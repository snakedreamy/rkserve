use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, atomic::{AtomicU64, Ordering}},
    time::{SystemTime, UNIX_EPOCH},
};

use rkserve_protocol::plugin::v1::ExecuteJobResponse;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex as AsyncMutex, Notify, RwLock, Semaphore};

use crate::{
    event_log::{EventSeverity, EventStore},
    supervisor::Supervisor,
};

// minimal-debt: edge deployments retain few terminal jobs, so linear scans are
// acceptable; replace with timestamp-indexed LRU once counts reach hundreds.
const MAX_RETAINED_TERMINAL_JOBS: usize = 64;
static JOB_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Canceling,
    Succeeded,
    Failed,
    Canceled,
}

impl JobState {
    pub(crate) fn is_active(self) -> bool { matches!(self, Self::Queued | Self::Running | Self::Canceling) }
    fn is_terminal(self) -> bool { matches!(self, Self::Succeeded | Self::Failed | Self::Canceled) }
    pub(crate) fn as_str(self) -> &'static str {
        match self { Self::Queued => "queued", Self::Running => "running", Self::Canceling => "canceling", Self::Succeeded => "succeeded", Self::Failed => "failed", Self::Canceled => "canceled" }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobTimings {
    pub queue_us: u64,
    pub preprocess_us: u64,
    pub inference_us: u64,
    pub postprocess_us: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSnapshot {
    pub id: String,
    pub plugin_id: String,
    pub capability_id: String,
    pub state: JobState,
    pub created_at_unix_ms: u64,
    pub started_at_unix_ms: Option<u64>,
    pub finished_at_unix_ms: Option<u64>,
    pub cancel_requested: bool,
    pub content_type: Option<String>,
    pub result_bytes: Option<usize>,
    pub timings: Option<JobTimings>,
    pub error: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub submitted_by: Option<String>,
    #[serde(default)]
    pub input_sha256: Option<String>,
    #[serde(default)]
    pub result_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredJob {
    pub snapshot: JobSnapshot,
    pub result_path: Option<PathBuf>,
}

struct JobEntry {
    snapshot: JobSnapshot,
    result_path: Option<PathBuf>,
    cancel_signal: Arc<Notify>,
}

struct JobManagerInner {
    jobs: RwLock<HashMap<String, JobEntry>>,
    output_root: PathBuf,
    supervisor: Supervisor,
    events: EventStore,
    plugin_slots: Mutex<HashMap<String, Arc<Semaphore>>>,
    // Serializes DB-backed transitions without holding the jobs lock across SQLite I/O.
    persistence: AsyncMutex<()>,
}

#[derive(Clone)]
pub struct JobManager { inner: Arc<JobManagerInner> }

#[derive(Debug, Error)]
pub enum JobError {
    #[error("job queue is full")]
    QueueFull,
    #[error("job '{0}' was not found")]
    NotFound(String),
    #[error("job result is not ready")]
    ResultNotReady,
    #[error("job is already complete")]
    AlreadyComplete,
    #[error("job result storage failed: {0}")]
    Storage(String),
}

pub struct JobRequest {
    pub plugin_id: String,
    pub capability_id: String,
    pub content_type: String,
    pub expected_output_content_type: String,
    pub payload: Vec<u8>,
    pub parameters: HashMap<String, String>,
    pub max_active: usize,
    pub max_concurrency: usize,
    pub request_id: String,
    pub submitted_by: String,
    pub input_sha256: String,
}

pub struct JobResult { pub snapshot: JobSnapshot, pub payload: Vec<u8> }

impl JobManager {
    pub async fn open(output_root: PathBuf, supervisor: Supervisor, events: EventStore) -> Result<Self, JobError> {
        tokio::fs::create_dir_all(&output_root).await.map_err(storage_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&output_root, std::fs::Permissions::from_mode(0o700))
                .await
                .map_err(storage_error)?;
        }
        let manager = Self { inner: Arc::new(JobManagerInner {
            jobs: RwLock::new(HashMap::new()), output_root, supervisor, events,
            plugin_slots: Mutex::new(HashMap::new()), persistence: AsyncMutex::new(()),
        }) };
        manager.restore().await?;
        Ok(manager)
    }

    async fn restore(&self) -> Result<(), JobError> {
        let mut stored = self.inner.events.recover_jobs().await.map_err(storage_error)?;
        stored.sort_by_key(|job| job.snapshot.created_at_unix_ms);
        let terminal_count = stored.iter().filter(|job| job.snapshot.state.is_terminal()).count();
        let remove_count = terminal_count.saturating_sub(MAX_RETAINED_TERMINAL_JOBS);
        let mut removed = Vec::new();
        let mut jobs = HashMap::new();
        for job in stored {
            if job.snapshot.state.is_terminal() && removed.len() < remove_count {
                removed.push(job);
                continue;
            }
            let id = job.snapshot.id.clone();
            jobs.insert(id, JobEntry { snapshot: job.snapshot, result_path: job.result_path, cancel_signal: Arc::new(Notify::new()) });
        }
        if !removed.is_empty() {
            self.inner.events.delete_jobs(removed.iter().map(|job| job.snapshot.id.clone()).collect()).await.map_err(storage_error)?;
            for job in removed {
                remove_job_files(&self.inner.output_root, &job.snapshot.id, job.result_path.as_deref()).await;
            }
        }
        *self.inner.jobs.write().await = jobs;
        Ok(())
    }

    pub async fn submit(&self, request: JobRequest) -> Result<JobSnapshot, JobError> {
        let _persist = self.inner.persistence.lock().await;
        let id = format!("job-{}-{:06}", unix_time_millis(), JOB_SEQUENCE.fetch_add(1, Ordering::Relaxed));
        let snapshot = JobSnapshot {
            id: id.clone(), plugin_id: request.plugin_id.clone(), capability_id: request.capability_id.clone(),
            state: JobState::Queued, created_at_unix_ms: unix_time_millis(), started_at_unix_ms: None,
            finished_at_unix_ms: None, cancel_requested: false, content_type: None, result_bytes: None,
            timings: None, error: None, request_id: Some(request.request_id.clone()),
            submitted_by: Some(request.submitted_by.clone()), input_sha256: Some(request.input_sha256.clone()),
            result_sha256: None,
        };
        let removals = {
            let jobs = self.inner.jobs.read().await;
            let active = jobs.values().filter(|entry| entry.snapshot.plugin_id == request.plugin_id && entry.snapshot.state.is_active()).count();
            if active >= request.max_active.max(1) { return Err(JobError::QueueFull); }
            let mut terminal: Vec<_> = jobs.values().filter(|entry| entry.snapshot.state.is_terminal())
                .map(|entry| (entry.snapshot.finished_at_unix_ms.unwrap_or(0), entry.snapshot.id.clone(), entry.result_path.clone())).collect();
            terminal.sort_unstable_by_key(|item| item.0);
            let needed = terminal.len().saturating_add(1).saturating_sub(MAX_RETAINED_TERMINAL_JOBS);
            terminal.into_iter().take(needed).collect::<Vec<_>>()
        };
        let request_path = self.request_path(&id);
        let temporary = self.inner.output_root.join(format!(".{id}.request.tmp"));
        write_private_file(&temporary, &request.payload).await?;
        if let Err(error) = tokio::fs::rename(&temporary, &request_path).await {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(storage_error(error));
        }
        if let Err(error) = self
            .inner
            .events
            .upsert_job(StoredJob {
                snapshot: snapshot.clone(),
                result_path: None,
            })
            .await
        {
            let _ = tokio::fs::remove_file(&request_path).await;
            return Err(storage_error(error));
        }
        if !removals.is_empty() {
            self.inner.events.delete_jobs(removals.iter().map(|(_, id, _)| id.clone()).collect()).await.map_err(storage_error)?;
        }
        {
            let mut jobs = self.inner.jobs.write().await;
            for (_, old_id, _) in &removals { jobs.remove(old_id); }
            jobs.insert(id.clone(), JobEntry { snapshot: snapshot.clone(), result_path: None, cancel_signal: Arc::new(Notify::new()) });
        }
        for (_, old_id, result_path) in &removals { remove_job_files(&self.inner.output_root, old_id, result_path.as_deref()).await; }
        drop(_persist);

        let execution_slots = {
            // This synchronous mutex is held only for the map lookup. Never
            // add an .await while the guard is alive.
            let mut slots = self.inner.plugin_slots.lock().unwrap_or_else(|error| error.into_inner());
            slots.entry(request.plugin_id.clone()).or_insert_with(|| Arc::new(Semaphore::new(request.max_concurrency.max(1)))).clone()
        };
        let cancel_signal = self.inner.jobs.read().await.get(&id).expect("job just inserted").cancel_signal.clone();
        self.inner.events.record_best_effort(
            EventSeverity::Info,
            "job",
            "submitted",
            "Asynchronous inference job submitted",
            Some(serde_json::json!({
                "capability_id": request.capability_id.clone(),
                "input_bytes": request.payload.len(),
                "input_sha256": request.input_sha256.clone(),
                "parameter_keys": request.parameters.keys().collect::<Vec<_>>(),
            })),
            Some(request.plugin_id.clone()),
            None,
            Some(id.clone()),
            Some(request.request_id.clone()),
            Some(request.submitted_by.clone()),
        ).await;
        let inner = self.inner.clone();
        tokio::spawn(async move { run_job(inner, id, request, execution_slots, cancel_signal).await });
        Ok(snapshot)
    }

    pub async fn get(&self, plugin_id: &str, id: &str) -> Result<JobSnapshot, JobError> {
        self.inner.jobs.read().await.get(id).filter(|entry| entry.snapshot.plugin_id == plugin_id)
            .map(|entry| entry.snapshot.clone()).ok_or_else(|| JobError::NotFound(id.to_owned()))
    }

    pub async fn cancel(&self, plugin_id: &str, id: &str) -> Result<JobSnapshot, JobError> {
        let _persist = self.inner.persistence.lock().await;
        let (next, signal) = {
            let jobs = self.inner.jobs.read().await;
            let entry = jobs.get(id).filter(|entry| entry.snapshot.plugin_id == plugin_id).ok_or_else(|| JobError::NotFound(id.to_owned()))?;
            if entry.snapshot.state.is_terminal() { return Err(JobError::AlreadyComplete); }
            let mut next = entry.snapshot.clone();
            next.cancel_requested = true;
            next.state = JobState::Canceling;
            (next, entry.cancel_signal.clone())
        };
        self.inner
            .events
            .upsert_job(StoredJob {
                snapshot: next.clone(),
                result_path: None,
            })
            .await
            .map_err(storage_error)?;
        if let Some(entry) = self.inner.jobs.write().await.get_mut(id) { entry.snapshot = next.clone(); }
        signal.notify_one();
        Ok(next)
    }

    pub async fn result(&self, plugin_id: &str, id: &str) -> Result<JobResult, JobError> {
        let (snapshot, path) = {
            let jobs = self.inner.jobs.read().await;
            let entry = jobs.get(id).filter(|entry| entry.snapshot.plugin_id == plugin_id).ok_or_else(|| JobError::NotFound(id.to_owned()))?;
            if entry.snapshot.state != JobState::Succeeded { return Err(JobError::ResultNotReady); }
            (entry.snapshot.clone(), entry.result_path.clone().ok_or(JobError::ResultNotReady)?)
        };
        let payload = tokio::fs::read(path).await.map_err(storage_error)?;
        Ok(JobResult { snapshot, payload })
    }

    pub async fn cancel_all(&self) {
        let active: Vec<_> = self.inner.jobs.read().await.values().filter(|entry| entry.snapshot.state.is_active())
            .map(|entry| (entry.snapshot.plugin_id.clone(), entry.snapshot.id.clone())).collect();
        for (plugin_id, id) in active { let _ = self.cancel(&plugin_id, &id).await; }
    }

    fn request_path(&self, id: &str) -> PathBuf { self.inner.output_root.join(format!("{id}.request")) }
}

async fn run_job(inner: Arc<JobManagerInner>, id: String, request: JobRequest, execution_slots: Arc<Semaphore>, cancel_signal: Arc<Notify>) {
    let permit = tokio::select! {
        permit = execution_slots.acquire_owned() => match permit { Ok(permit) => permit, Err(_) => { fail_job(&inner, &id, &request.plugin_id, "job execution slots closed".into()).await; return; } },
        _ = cancel_signal.notified() => {
            let _persist = inner.persistence.lock().await;
            let entry = inner.jobs.read().await.get(&id)
                .map(|entry| (entry.snapshot.clone(), entry.result_path.clone()));
            if let Some((snapshot, path)) = entry {
                let _ = finish_canceled(&inner, &id, snapshot, path).await;
            }
            record_job_event(&inner, EventSeverity::Info, "canceled", "Queued inference job canceled", &id, &request.plugin_id, None).await;
            return;
        }
    };
    match mark_running(&inner, &id).await {
        Ok(true) => {}
        Ok(false) => {
            record_job_event(&inner, EventSeverity::Info, "canceled", "Queued inference job canceled", &id, &request.plugin_id, None).await;
            return;
        }
        Err(error) => {
            fail_job(&inner, &id, &request.plugin_id, error.to_string()).await;
            return;
        }
    }
    let response = inner.supervisor.execute_job(&request.plugin_id, id.clone(), request.capability_id, request.content_type, request.payload, request.parameters).await;
    drop(permit);
    let mut response = match response { Ok(response) => response, Err(error) => { fail_job(&inner, &id, &request.plugin_id, error.to_string()).await; return; } };
    let queue_us = inner.jobs.read().await.get(&id).and_then(|entry| entry.snapshot.started_at_unix_ms.map(|started| started.saturating_sub(entry.snapshot.created_at_unix_ms) * 1_000)).unwrap_or(0);
    response.queue_us = response.queue_us.saturating_add(queue_us);
    if response.content_type.split(';').next().unwrap_or(&response.content_type).trim() != request.expected_output_content_type {
        fail_job(&inner, &id, &request.plugin_id, format!("worker returned '{}', expected '{}'", response.content_type, request.expected_output_content_type)).await;
        return;
    }
    if let Err(error) = store_result(&inner, &id, &request.plugin_id, response).await { fail_job(&inner, &id, &request.plugin_id, error.to_string()).await; }
}

async fn mark_running(inner: &Arc<JobManagerInner>, id: &str) -> Result<bool, JobError> {
    let _persist = inner.persistence.lock().await;
    let (mut snapshot, path) = { let jobs = inner.jobs.read().await; let entry = jobs.get(id).ok_or_else(|| JobError::NotFound(id.into()))?; (entry.snapshot.clone(), entry.result_path.clone()) };
    if snapshot.cancel_requested {
        finish_canceled(inner, id, snapshot, path).await?;
        return Ok(false);
    }
    snapshot.state = JobState::Running; snapshot.started_at_unix_ms = Some(unix_time_millis());
    inner.events.upsert_job(StoredJob { snapshot: snapshot.clone(), result_path: path }).await.map_err(storage_error)?;
    if let Some(entry) = inner.jobs.write().await.get_mut(id) { entry.snapshot = snapshot; }
    Ok(true)
}

async fn store_result(inner: &Arc<JobManagerInner>, id: &str, plugin_id: &str, response: ExecuteJobResponse) -> Result<(), JobError> {
    let temporary = inner.output_root.join(format!(".{id}.tmp"));
    let destination = inner.output_root.join(format!("{id}.bin"));
    write_private_file(&temporary, &response.payload).await?;
    tokio::fs::rename(&temporary, &destination).await.map_err(storage_error)?;
    let _persist = inner.persistence.lock().await;
    let (mut snapshot, previous_path) = { let jobs = inner.jobs.read().await; let entry = jobs.get(id).ok_or_else(|| JobError::NotFound(id.into()))?; (entry.snapshot.clone(), entry.result_path.clone()) };
    if snapshot.cancel_requested {
        finish_canceled(inner, id, snapshot, previous_path).await?;
        let _ = tokio::fs::remove_file(destination).await;
        return Ok(());
    }
    snapshot.state = JobState::Succeeded; snapshot.finished_at_unix_ms = Some(unix_time_millis()); snapshot.content_type = Some(response.content_type); snapshot.result_bytes = Some(response.payload.len());
    snapshot.result_sha256 = Some(sha256_hex(&response.payload));
    snapshot.timings = Some(JobTimings { queue_us: response.queue_us, preprocess_us: response.preprocess_us, inference_us: response.inference_us, postprocess_us: response.postprocess_us });
    inner.events.upsert_job(StoredJob { snapshot: snapshot.clone(), result_path: Some(destination.clone()) }).await.map_err(storage_error)?;
    if let Some(entry) = inner.jobs.write().await.get_mut(id) { entry.snapshot = snapshot; entry.result_path = Some(destination); }
    record_job_event(inner, EventSeverity::Info, "succeeded", "Asynchronous inference job completed", id, plugin_id, None).await;
    Ok(())
}

async fn fail_job(inner: &Arc<JobManagerInner>, id: &str, plugin_id: &str, error: String) {
    let _persist = inner.persistence.lock().await;
    let (mut snapshot, path) = match inner.jobs.read().await.get(id) { Some(entry) => (entry.snapshot.clone(), entry.result_path.clone()), None => return };
    let canceled = snapshot.cancel_requested;
    if canceled { snapshot.state = JobState::Canceled; } else { snapshot.state = JobState::Failed; snapshot.error = Some(error.clone()); }
    snapshot.finished_at_unix_ms = Some(unix_time_millis());
    if inner.events.upsert_job(StoredJob { snapshot: snapshot.clone(), result_path: path }).await.is_ok() {
        if let Some(entry) = inner.jobs.write().await.get_mut(id) { entry.snapshot = snapshot; }
    }
    let (severity, kind, message, metadata) = if canceled { (EventSeverity::Info, "canceled", "Asynchronous inference job canceled", None) } else { (EventSeverity::Error, "failed", "Asynchronous inference job failed", Some(serde_json::json!({"error": error}))) };
    record_job_event(inner, severity, kind, message, id, plugin_id, metadata).await;
}

async fn finish_canceled(
    inner: &Arc<JobManagerInner>,
    id: &str,
    mut snapshot: JobSnapshot,
    path: Option<PathBuf>,
) -> Result<(), JobError> {
    snapshot.cancel_requested = true;
    snapshot.state = JobState::Canceled;
    snapshot.finished_at_unix_ms = Some(unix_time_millis());
    inner
        .events
        .upsert_job(StoredJob {
            snapshot: snapshot.clone(),
            result_path: path,
        })
        .await
        .map_err(storage_error)?;
    if let Some(entry) = inner.jobs.write().await.get_mut(id) {
        entry.snapshot = snapshot;
    }
    Ok(())
}

async fn record_job_event(inner: &Arc<JobManagerInner>, severity: EventSeverity, kind: &str, message: &str, id: &str, plugin_id: &str, metadata: Option<serde_json::Value>) {
    let snapshot = inner.jobs.read().await.get(id).map(|entry| entry.snapshot.clone());
    let details = if let Some(snapshot) = snapshot.as_ref() {
        let mut details = metadata.unwrap_or_else(|| serde_json::json!({}));
        if let Some(object) = details.as_object_mut() {
            object.insert("capability_id".into(), snapshot.capability_id.clone().into());
            object.insert("input_sha256".into(), snapshot.input_sha256.clone().into());
            object.insert("result_sha256".into(), snapshot.result_sha256.clone().into());
            object.insert("result_bytes".into(), snapshot.result_bytes.into());
            object.insert("timings".into(), serde_json::to_value(&snapshot.timings).unwrap_or_default());
        }
        Some(details)
    } else {
        metadata
    };
    inner.events.record_best_effort(
        severity,
        "job",
        kind,
        message,
        details,
        Some(plugin_id.into()),
        None,
        Some(id.into()),
        snapshot.as_ref().and_then(|snapshot| snapshot.request_id.clone()),
        snapshot.as_ref().and_then(|snapshot| snapshot.submitted_by.clone()).or_else(|| Some("core".into())),
    ).await;
}

async fn remove_job_files(output_root: &std::path::Path, id: &str, result_path: Option<&std::path::Path>) {
    let _ = tokio::fs::remove_file(output_root.join(format!("{id}.request"))).await;
    if let Some(path) = result_path { let _ = tokio::fs::remove_file(path).await; } else { let _ = tokio::fs::remove_file(output_root.join(format!("{id}.bin"))).await; }
}

async fn write_private_file(path: &std::path::Path, payload: &[u8]) -> Result<(), JobError> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options.open(path).await.map_err(storage_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .await
            .map_err(storage_error)?;
    }
    file.write_all(payload).await.map_err(storage_error)?;
    file.flush().await.map_err(storage_error)
}

fn storage_error(error: impl std::fmt::Display) -> JobError { JobError::Storage(error.to_string()) }
fn unix_time_millis() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64 }
fn sha256_hex(payload: &[u8]) -> String {
    Sha256::digest(payload).iter().map(|byte| format!("{byte:02x}")).collect()
}
