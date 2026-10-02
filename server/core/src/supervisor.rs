use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use http::Uri;
use hyper_util::rt::TokioIo;
use rkserve_protocol::{
    plugin::v1::{
        DescribeRequest, DrainRequest, ExecuteJobRequest, ExecuteJobResponse, HealthRequest, LoadRequest,
        NpuCoreMask, UnloadRequest, health_response::ServingStatus,
        plugin_runtime_client::PluginRuntimeClient,
    },
    PROTOCOL_VERSION,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    net::UnixStream,
    process::{Child, Command},
    sync::{mpsc, oneshot, Mutex, RwLock},
    time::{sleep, timeout, Instant},
};
use tonic::{Code, Status, transport::{Channel, Endpoint}};
use tower::service_fn;
use tracing::{error, info, warn};

use crate::{
    domain::{Allocation, CoreMask},
    event_log::{EventSeverity, EventStore},
    registry::PluginDefinition,
    scheduler::Scheduler,
    state_store::StateStore,
};

const START_TIMEOUT: Duration = Duration::from_secs(20);
const RPC_TIMEOUT: Duration = Duration::from_secs(15);
const STOP_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_RESTART_ATTEMPTS: u8 = 5;
const MAX_RPC_MESSAGE_SIZE: usize = 64 * 1024 * 1024;

type Client = PluginRuntimeClient<Channel>;

#[derive(Debug, Error)]
pub enum SupervisorError {
    #[error("plugin '{0}' is already running")]
    AlreadyRunning(String),
    #[error("failed to start plugin process: {0}")]
    Spawn(String),
    #[error("plugin did not create its socket before the startup deadline")]
    SocketTimeout,
    #[error("could not connect to plugin: {0}")]
    Connect(String),
    #[error("plugin handshake failed: {0}")]
    Handshake(String),
    #[error("plugin identity does not match its manifest")]
    IdentityMismatch,
    #[error("plugin protocol version is not supported")]
    ProtocolMismatch,
    #[error("worker does not report NPU execution capability")]
    NpuRequired,
    #[error("worker loaded an unexpected NPU core mask")]
    CoreMaskMismatch,
    #[error("worker capabilities do not match its manifest")]
    CapabilityMismatch,
    #[error("worker model target is not compatible with RK3576")]
    ModelTargetMismatch,
    #[error("plugin worker for lease '{0}' was not found")]
    WorkerNotFound(String),
    #[error("plugin worker is temporarily unavailable")]
    CommandUnavailable,
    #[error("plugin inference failed: {0}")]
    Inference(String),
    #[error("plugin rejected the request: {0}")]
    InvalidInput(String),
    #[error("plugin worker did not stop before the deadline")]
    StopTimeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerStatus {
    Ready,
    Backoff,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkerSnapshot {
    pub lease_id: String,
    pub plugin_id: String,
    pub pid: Option<u32>,
    pub runtime_version: String,
    pub driver_version: String,
    pub model_target: String,
    pub core_mask: CoreMask,
    pub status: WorkerStatus,
    pub restart_attempt: u8,
    pub last_error: Option<String>,
}

enum StopMode {
    Manual,
    Shutdown,
}

enum WorkerCommand {
    ExecuteJob {
        request: ExecuteJobRequest,
        response: oneshot::Sender<Result<ExecuteJobResponse, SupervisorError>>,
    },
    Stop {
        mode: StopMode,
        response: oneshot::Sender<Result<(), SupervisorError>>,
    },
}

struct WorkerHandle {
    // This public-facing snapshot is synchronized from RunningWorker only in
    // set_snapshot/update_snapshot; new lifecycle transitions must use them.
    snapshot: WorkerSnapshot,
    commands: mpsc::Sender<WorkerCommand>,
    request_timeout: Duration,
}

struct RunningWorker {
    child: Child,
    client: Client,
    socket_path: PathBuf,
    marker_path: PathBuf,
    snapshot: WorkerSnapshot,
}

struct SupervisorInner {
    runtime_root: PathBuf,
    workers: RwLock<HashMap<String, WorkerHandle>>,
    // ponytail: global start lock; use per-plugin locks only if parallel startup becomes necessary.
    start_lock: Mutex<()>,
    scheduler: Arc<RwLock<Scheduler>>,
    events: EventStore,
    state_store: StateStore,
}

#[derive(Clone)]
pub struct Supervisor {
    inner: Arc<SupervisorInner>,
}

impl Supervisor {
    pub fn new(
        runtime_root: PathBuf,
        scheduler: Arc<RwLock<Scheduler>>,
        events: EventStore,
        state_store: StateStore,
    ) -> Self {
        let runtime_root = absolute_path(runtime_root);
        Self {
            inner: Arc::new(SupervisorInner {
                runtime_root,
                workers: RwLock::new(HashMap::new()),
                start_lock: Mutex::new(()),
                scheduler,
                events,
                state_store,
            }),
        }
    }

    pub async fn reconcile_runtime(&self) {
        if let Err(error) = tokio::fs::create_dir_all(&self.inner.runtime_root).await {
            warn!(%error, "failed to create runtime directory during reconciliation");
            return;
        }
        let mut entries = match tokio::fs::read_dir(&self.inner.runtime_root).await {
            Ok(entries) => entries,
            Err(error) => {
                warn!(%error, "failed to inspect runtime directory");
                return;
            }
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            match path.extension().and_then(|value| value.to_str()) {
                Some("json") if path.file_name().and_then(|value| value.to_str()).is_some_and(|name| name.ends_with(".worker.json")) => {
                    cleanup_stale_marker(&path, &self.inner.events).await;
                }
                Some("sock") => remove_file(&path).await,
                _ => {}
            }
        }
        record(
            &self.inner.events,
            SupervisorEvent {
                severity: EventSeverity::Info,
                category: "system",
                kind: "runtime_reconciled",
                message: "Runtime directory reconciliation completed",
                detail: None,
                plugin_id: None,
                lease_id: None,
            },
        ).await;
    }

    pub async fn start(
        &self,
        plugin: &PluginDefinition,
        allocation: &Allocation,
    ) -> Result<WorkerSnapshot, SupervisorError> {
        let _start = self.inner.start_lock.lock().await;
        if self.inner.workers.read().await.values().any(|worker| worker.snapshot.plugin_id == plugin.summary.id) {
            return Err(SupervisorError::AlreadyRunning(plugin.summary.id.clone()));
        }
        tokio::fs::create_dir_all(&self.inner.runtime_root)
            .await
            .map_err(|error| SupervisorError::Spawn(error.to_string()))?;

        let supplied = self.inner.state_store.get(&plugin.summary.id).map(|state| state.configuration).unwrap_or_default();
        let configuration = plugin.resolve_configuration(&supplied).map_err(|error| SupervisorError::Spawn(error.to_string()))?;
        let running = match spawn_worker(&self.inner.runtime_root, plugin, allocation, &configuration, 0).await {
            Ok(worker) => worker,
            Err(error) => {
                record(
                    &self.inner.events,
                    SupervisorEvent {
                        severity: EventSeverity::Error,
                        category: "worker",
                        kind: "start_failed",
                        message: "Plugin worker failed to start",
                        detail: Some(error.to_string()),
                        plugin_id: Some(plugin.summary.id.clone()),
                        lease_id: Some(allocation.lease_id.clone()),
                    },
                ).await;
                return Err(error);
            }
        };
        let snapshot = running.snapshot.clone();
        let (commands, receiver) = mpsc::channel(plugin.summary.queue_size.clamp(1, 256) as usize);
        let request_timeout = plugin_request_timeout(plugin);
        self.inner.workers.write().await.insert(allocation.lease_id.clone(), WorkerHandle { snapshot: snapshot.clone(), commands, request_timeout });
        record(
            &self.inner.events,
            SupervisorEvent {
                severity: EventSeverity::Info,
                category: "worker",
                kind: "ready",
                message: "Plugin worker is ready",
                detail: Some(format!(
                    "PID {} · {}",
                    snapshot.pid.unwrap_or_default(),
                    snapshot.core_mask.label()
                )),
                plugin_id: Some(snapshot.plugin_id.clone()),
                lease_id: Some(snapshot.lease_id.clone()),
            },
        ).await;

        tokio::spawn(worker_manager(self.inner.clone(), plugin.clone(), allocation.clone(), configuration, running, receiver));
        info!(plugin_id = %snapshot.plugin_id, lease_id = %snapshot.lease_id, pid = ?snapshot.pid, core_mask = ?snapshot.core_mask, "plugin worker is ready");
        Ok(snapshot)
    }

    pub async fn stop(&self, lease_id: &str) -> Result<(), SupervisorError> {
        self.stop_with_mode(lease_id, StopMode::Manual).await
    }

    async fn stop_with_mode(&self, lease_id: &str, mode: StopMode) -> Result<(), SupervisorError> {
        let commands = self.inner.workers.read().await.get(lease_id).map(|worker| worker.commands.clone())
            .ok_or_else(|| SupervisorError::WorkerNotFound(lease_id.to_owned()))?;
        let (response, result) = oneshot::channel();
        commands.send(WorkerCommand::Stop { mode, response }).await.map_err(|_| SupervisorError::CommandUnavailable)?;
        timeout(STOP_TIMEOUT, result).await.map_err(|_| SupervisorError::StopTimeout)?
            .map_err(|_| SupervisorError::CommandUnavailable)??;
        Ok(())
    }

    pub async fn execute_job(&self, plugin_id: &str, job_id: String, capability_id: String, content_type: String, payload: Vec<u8>, parameters: HashMap<String, String>) -> Result<ExecuteJobResponse, SupervisorError> {
        let (commands, request_timeout) = self.inner.workers.read().await.values().find(|worker| worker.snapshot.plugin_id == plugin_id)
            .map(|worker| (worker.commands.clone(), worker.request_timeout)).ok_or_else(|| SupervisorError::WorkerNotFound(plugin_id.to_owned()))?;
        let request = ExecuteJobRequest {
            job_id, request_id: format!("request-{}", unix_time_micros()), capability_id, content_type, payload, parameters,
        };
        let (response, result) = oneshot::channel();
        commands.send(WorkerCommand::ExecuteJob { request, response }).await.map_err(|_| SupervisorError::CommandUnavailable)?;
        timeout(request_timeout + Duration::from_secs(1), result).await.map_err(|_| SupervisorError::Inference("request timed out".into()))?
            .map_err(|_| SupervisorError::CommandUnavailable)?
    }

    pub async fn snapshots(&self) -> Vec<WorkerSnapshot> {
        let mut snapshots: Vec<_> = self.inner.workers.read().await.values().map(|worker| worker.snapshot.clone()).collect();
        snapshots.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
        snapshots
    }

    pub async fn shutdown_all(&self) {
        let lease_ids: Vec<_> = self.inner.workers.read().await.keys().cloned().collect();
        for lease_id in lease_ids {
            if let Err(error) = self.stop_with_mode(&lease_id, StopMode::Shutdown).await {
                warn!(%lease_id, %error, "failed to stop plugin during shutdown");
            }
        }
    }
}

enum SessionExit {
    Stop(StopMode, oneshot::Sender<Result<(), SupervisorError>>, Result<(), SupervisorError>),
    Unexpected(String),
}

async fn worker_manager(
    inner: Arc<SupervisorInner>,
    plugin: PluginDefinition,
    allocation: Allocation,
    configuration: BTreeMap<String, String>,
    mut running: RunningWorker,
    mut commands: mpsc::Receiver<WorkerCommand>,
) {
    let mut restart_attempt = 0_u8;
    loop {
        let exit = worker_session(&mut running, &mut commands, plugin_request_timeout(&plugin)).await;
        cleanup_worker_files(&running).await;
        match exit {
            SessionExit::Stop(mode, response, result) => {
                inner.workers.write().await.remove(&allocation.lease_id);
                let _ = inner.scheduler.write().await.release(&allocation.lease_id);
                let kind = match mode { StopMode::Manual => "stopped", StopMode::Shutdown => "shutdown" };
                record(
                    &inner.events,
                    SupervisorEvent {
                        severity: EventSeverity::Info,
                        category: "worker",
                        kind,
                        message: "Plugin worker stopped",
                        detail: None,
                        plugin_id: Some(plugin.summary.id.clone()),
                        lease_id: Some(allocation.lease_id.clone()),
                    },
                ).await;
                let _ = response.send(result);
                return;
            }
            SessionExit::Unexpected(mut reason) => {
                loop {
                    restart_attempt = restart_attempt.saturating_add(1);
                    if restart_attempt > MAX_RESTART_ATTEMPTS {
                        inner.workers.write().await.remove(&allocation.lease_id);
                        let _ = inner.scheduler.write().await.release(&allocation.lease_id);
                        let error_message = format!("Failed to restart after {MAX_RESTART_ATTEMPTS} consecutive attempts: {reason}");
                        if let Err(error) = inner
                            .state_store
                            .fail(
                                &plugin.summary.id,
                                allocation.core_mask,
                                error_message.clone(),
                            )
                            .await
                        {
                            error!(%error, "failed to persist circuit breaker state");
                        }
                        record(
                            &inner.events,
                            SupervisorEvent {
                                severity: EventSeverity::Error,
                                category: "supervisor",
                                kind: "circuit_open",
                                message: "Plugin circuit breaker opened; NPU lease released",
                                detail: Some(error_message),
                                plugin_id: Some(plugin.summary.id.clone()),
                                lease_id: Some(allocation.lease_id.clone()),
                            },
                        ).await;
                        return;
                    }

                    let delay = Duration::from_secs((1_u64 << (restart_attempt - 1)).min(30));
                    update_snapshot(&inner, &allocation.lease_id, WorkerStatus::Backoff, restart_attempt, Some(reason.clone())).await;
                    record(
                        &inner.events,
                        SupervisorEvent {
                            severity: EventSeverity::Warning,
                            category: "supervisor",
                            kind: "restart_scheduled",
                            message: "Worker failed; automatic restart scheduled",
                            detail: Some(format!(
                                "Attempt {restart_attempt}/{MAX_RESTART_ATTEMPTS} · in {} seconds · {reason}",
                                delay.as_secs()
                            )),
                            plugin_id: Some(plugin.summary.id.clone()),
                            lease_id: Some(allocation.lease_id.clone()),
                        },
                    ).await;

                    match wait_backoff(delay, &mut commands).await {
                        Some((mode, response)) => {
                            inner.workers.write().await.remove(&allocation.lease_id);
                            let _ = inner.scheduler.write().await.release(&allocation.lease_id);
                            let _ = response.send(Ok(()));
                            let kind = match mode { StopMode::Manual => "stopped", StopMode::Shutdown => "shutdown" };
                            record(
                                &inner.events,
                                SupervisorEvent {
                                    severity: EventSeverity::Info,
                                    category: "worker",
                                    kind,
                                    message: "Plugin stopped while waiting to restart",
                                    detail: None,
                                    plugin_id: Some(plugin.summary.id.clone()),
                                    lease_id: Some(allocation.lease_id.clone()),
                                },
                            ).await;
                            return;
                        }
                        None => {}
                    }

                    match spawn_worker(&inner.runtime_root, &plugin, &allocation, &configuration, restart_attempt).await {
                        Ok(next) => {
                            running = next;
                            set_snapshot(&inner, &allocation.lease_id, running.snapshot.clone()).await;
                            record(
                                &inner.events,
                                SupervisorEvent {
                                    severity: EventSeverity::Info,
                                    category: "supervisor",
                                    kind: "restarted",
                                    message: "Plugin worker recovered automatically",
                                    detail: Some(format!(
                                        "Attempt {restart_attempt} · PID {}",
                                        running.snapshot.pid.unwrap_or_default()
                                    )),
                                    plugin_id: Some(plugin.summary.id.clone()),
                                    lease_id: Some(allocation.lease_id.clone()),
                                },
                            ).await;
                            break;
                        }
                        Err(error) => {
                            reason = error.to_string();
                            record(
                                &inner.events,
                                SupervisorEvent {
                                    severity: EventSeverity::Error,
                                    category: "supervisor",
                                    kind: "restart_failed",
                                    message: "Plugin worker automatic restart failed",
                                    detail: Some(reason.clone()),
                                    plugin_id: Some(plugin.summary.id.clone()),
                                    lease_id: Some(allocation.lease_id.clone()),
                                },
                            ).await;
                        }
                    }
                }
            }
        }
    }
}

async fn worker_session(running: &mut RunningWorker, commands: &mut mpsc::Receiver<WorkerCommand>, request_timeout: Duration) -> SessionExit {
    let lease_id = running.snapshot.lease_id.clone();
    let mut health = tokio::time::interval(Duration::from_secs(5));
    health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut health_failures = 0_u8;
    loop {
        tokio::select! {
            exit = running.child.wait() => {
                let reason = match exit { Ok(status) => format!("Process exited: {status}"), Err(error) => format!("Failed to wait for process: {error}") };
                error!(%lease_id, reason, "plugin worker exited unexpectedly");
                return SessionExit::Unexpected(reason);
            }
            _ = health.tick() => {
                let serving = timeout(Duration::from_secs(3), running.client.health(HealthRequest {})).await.ok()
                    .and_then(Result::ok).map(|response| response.into_inner().status == ServingStatus::Serving as i32).unwrap_or(false);
                if serving { health_failures = 0; } else {
                    health_failures += 1;
                    warn!(%lease_id, health_failures, "plugin worker health check failed");
                    if health_failures >= 3 {
                        terminate_child(&mut running.child).await;
                        return SessionExit::Unexpected("Three consecutive health checks failed".into());
                    }
                }
            }
            command = commands.recv() => match command {
                Some(WorkerCommand::ExecuteJob { request, response }) => {
                    let result = timeout(request_timeout, running.client.execute_job(request)).await
                        .map_err(|_| SupervisorError::Inference("worker timed out".into()))
                        .and_then(|result| result.map(|response| response.into_inner()).map_err(inference_error));
                    let _ = response.send(result);
                }
                Some(WorkerCommand::Stop { mode, response }) => {
                    let result = stop_worker(&mut running.client, &mut running.child).await;
                    return SessionExit::Stop(mode, response, result);
                }
                None => {
                    terminate_child(&mut running.child).await;
                    return SessionExit::Unexpected("Supervisor command channel closed".into());
                }
            }
        }
    }
}

async fn wait_backoff(delay: Duration, commands: &mut mpsc::Receiver<WorkerCommand>) -> Option<(StopMode, oneshot::Sender<Result<(), SupervisorError>>)> {
    let deadline = Instant::now() + delay;
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => return None,
            command = commands.recv() => match command {
                Some(WorkerCommand::ExecuteJob { response, .. }) => { let _ = response.send(Err(SupervisorError::CommandUnavailable)); }
                Some(WorkerCommand::Stop { mode, response }) => return Some((mode, response)),
                None => return None,
            }
        }
    }
}

async fn spawn_worker(runtime_root: &Path, plugin: &PluginDefinition, allocation: &Allocation, configuration: &BTreeMap<String, String>, restart_attempt: u8) -> Result<RunningWorker, SupervisorError> {
    let socket_path = runtime_root.join(format!("{}.sock", allocation.lease_id));
    let marker_path = runtime_root.join(format!("{}.worker.json", allocation.lease_id));
    remove_file(&socket_path).await;
    remove_file(&marker_path).await;
    let mut child = Command::new(&plugin.executable)
        .current_dir(&plugin.plugin_dir)
        .env("RKSERVE_WORKER_SOCKET", &socket_path)
        .env("RKSERVE_PLUGIN_DIR", &plugin.plugin_dir)
        .env("RKSERVE_PLUGIN_ID", &plugin.summary.id)
        .stdin(Stdio::null()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).kill_on_drop(true)
        .spawn().map_err(|error| SupervisorError::Spawn(error.to_string()))?;

    let startup = handshake(plugin, allocation, configuration, &socket_path).await;
    let (client, mut snapshot) = match startup {
        Ok(result) => result,
        Err(error) => { terminate_child(&mut child).await; remove_file(&socket_path).await; return Err(error); }
    };
    snapshot.pid = child.id();
    snapshot.restart_attempt = restart_attempt;
    let marker = WorkerMarker { pid: child.id().unwrap_or_default(), plugin_id: plugin.summary.id.clone(), lease_id: allocation.lease_id.clone() };
    let marker_bytes = serde_json::to_vec(&marker).map_err(|error| SupervisorError::Spawn(error.to_string()))?;
    tokio::fs::write(&marker_path, marker_bytes).await.map_err(|error| SupervisorError::Spawn(error.to_string()))?;
    Ok(RunningWorker { child, client, socket_path, marker_path, snapshot })
}

async fn handshake(plugin: &PluginDefinition, allocation: &Allocation, configuration: &BTreeMap<String, String>, socket_path: &Path) -> Result<(Client, WorkerSnapshot), SupervisorError> {
    let mut client = connect_uds(socket_path).await?;
    let description = timeout(RPC_TIMEOUT, client.describe(DescribeRequest {})).await
        .map_err(|_| SupervisorError::Handshake("Describe timed out".into()))?
        .map_err(|error| SupervisorError::Handshake(error.to_string()))?.into_inner();
    if description.plugin_id != plugin.summary.id || description.plugin_version != plugin.summary.version { return Err(SupervisorError::IdentityMismatch); }
    if description.protocol_version != PROTOCOL_VERSION { return Err(SupervisorError::ProtocolMismatch); }
    if !description.npu_required { return Err(SupervisorError::NpuRequired); }
    if plugin.summary.capabilities.iter().any(|capability| !description.capabilities.iter().any(|reported| reported == &capability.id)) {
        return Err(SupervisorError::CapabilityMismatch);
    }
    let expected_mask = proto_mask(allocation.core_mask);
    let loaded = timeout(plugin_request_timeout(plugin).max(RPC_TIMEOUT), client.load(LoadRequest { lease_id: allocation.lease_id.clone(), core_mask: expected_mask, config: configuration.clone().into_iter().collect() })).await
        .map_err(|_| SupervisorError::Handshake("Load timed out".into()))?
        .map_err(|error| SupervisorError::Handshake(error.to_string()))?.into_inner();
    if loaded.active_core_mask != expected_mask { return Err(SupervisorError::CoreMaskMismatch); }
    if !loaded
        .model_target
        .to_ascii_lowercase()
        .contains(crate::PLATFORM)
    {
        return Err(SupervisorError::ModelTargetMismatch);
    }
    let health = timeout(RPC_TIMEOUT, client.health(HealthRequest {})).await
        .map_err(|_| SupervisorError::Handshake("Health timed out".into()))?
        .map_err(|error| SupervisorError::Handshake(error.to_string()))?.into_inner();
    if health.status != ServingStatus::Serving as i32 { return Err(SupervisorError::Handshake(format!("worker is not serving: {}", health.detail))); }
    Ok((client, WorkerSnapshot {
        lease_id: allocation.lease_id.clone(), plugin_id: plugin.summary.id.clone(), pid: None,
        runtime_version: loaded.runtime_version,
        driver_version: loaded.driver_version, model_target: loaded.model_target, core_mask: allocation.core_mask,
        status: WorkerStatus::Ready, restart_attempt: 0, last_error: None,
    }))
}

async fn connect_uds(socket_path: &Path) -> Result<Client, SupervisorError> {
    let deadline = Instant::now() + START_TIMEOUT;
    // minimal-debt: startup uses 50 ms polling; switch to filesystem events
    // only if worker startup latency or process count makes polling measurable.
    while !socket_path.exists() {
        if Instant::now() >= deadline { return Err(SupervisorError::SocketTimeout); }
        sleep(Duration::from_millis(50)).await;
    }
    let path = socket_path.to_owned();
    let channel = timeout(START_TIMEOUT, Endpoint::try_from("http://[::]:50051")
        .map_err(|error| SupervisorError::Connect(error.to_string()))?
        .connect_with_connector(service_fn(move |_: Uri| { let path = path.clone(); async move { UnixStream::connect(path).await.map(TokioIo::new) } })))
        .await.map_err(|_| SupervisorError::SocketTimeout)?.map_err(|error| SupervisorError::Connect(error.to_string()))?;
    Ok(PluginRuntimeClient::new(channel)
        .max_decoding_message_size(MAX_RPC_MESSAGE_SIZE)
        .max_encoding_message_size(MAX_RPC_MESSAGE_SIZE))
}

async fn stop_worker(client: &mut Client, child: &mut Child) -> Result<(), SupervisorError> {
    let _ = timeout(RPC_TIMEOUT, client.drain(DrainRequest { timeout_ms: 10_000 })).await;
    let _ = timeout(RPC_TIMEOUT, client.unload(UnloadRequest {})).await;
    terminate_child(child).await;
    Ok(())
}

async fn terminate_child(child: &mut Child) {
    if let Ok(Some(_)) = child.try_wait() { return; }
    if let Err(error) = child.start_kill() { warn!(%error, "failed to signal plugin worker"); }
    let _ = timeout(Duration::from_secs(5), child.wait()).await;
}

async fn cleanup_worker_files(worker: &RunningWorker) {
    remove_file(&worker.socket_path).await;
    remove_file(&worker.marker_path).await;
}

async fn remove_file(path: &Path) {
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => warn!(path = %path.display(), %error, "failed to remove runtime file"),
    }
}

#[derive(Serialize, Deserialize)]
struct WorkerMarker { pid: u32, plugin_id: String, lease_id: String }

async fn cleanup_stale_marker(path: &Path, events: &EventStore) {
    let marker = tokio::fs::read(path).await.ok().and_then(|raw| serde_json::from_slice::<WorkerMarker>(&raw).ok());
    if let Some(marker) = marker {
        let environment = tokio::fs::read(format!("/proc/{}/environ", marker.pid)).await.unwrap_or_default();
        let expected = format!("RKSERVE_PLUGIN_ID={}", marker.plugin_id);
        let owns_process = environment.split(|byte| *byte == 0).any(|item| item == expected.as_bytes());
        if owns_process {
            let _ = Command::new("kill").args(["-TERM", &marker.pid.to_string()]).status().await;
            sleep(Duration::from_millis(500)).await;
            if Path::new(&format!("/proc/{}", marker.pid)).exists() {
                let _ = Command::new("kill").args(["-KILL", &marker.pid.to_string()]).status().await;
            }
            record(
                events,
                SupervisorEvent {
                    severity: EventSeverity::Warning,
                    category: "system",
                    kind: "orphan_cleaned",
                    message: "Cleaned up a worker left over from the previous run",
                    detail: Some(format!("PID {}", marker.pid)),
                    plugin_id: Some(marker.plugin_id),
                    lease_id: Some(marker.lease_id),
                },
            ).await;
        }
    }
    remove_file(path).await;
}

async fn update_snapshot(inner: &SupervisorInner, lease_id: &str, status: WorkerStatus, attempt: u8, error: Option<String>) {
    if let Some(worker) = inner.workers.write().await.get_mut(lease_id) {
        worker.snapshot.status = status; worker.snapshot.restart_attempt = attempt; worker.snapshot.last_error = error; worker.snapshot.pid = None;
    }
}

async fn set_snapshot(inner: &SupervisorInner, lease_id: &str, snapshot: WorkerSnapshot) {
    if let Some(worker) = inner.workers.write().await.get_mut(lease_id) { worker.snapshot = snapshot; }
}

struct SupervisorEvent<'a> {
    severity: EventSeverity,
    category: &'a str,
    kind: &'a str,
    message: &'a str,
    detail: Option<String>,
    plugin_id: Option<String>,
    lease_id: Option<String>,
}

async fn record(events: &EventStore, event: SupervisorEvent<'_>) {
    events
        .record_best_effort(
            event.severity,
            event.category,
            event.kind,
            event.message,
            event
                .detail
                .map(|detail| serde_json::json!({"detail": detail})),
            event.plugin_id,
            event.lease_id,
            None,
            None,
            Some("core".into()),
        )
        .await;
}

fn proto_mask(mask: CoreMask) -> i32 {
    match mask { CoreMask::Core0 => NpuCoreMask::NpuCore0 as i32, CoreMask::Core1 => NpuCoreMask::NpuCore1 as i32, CoreMask::Core0_1 => NpuCoreMask::NpuCore01 as i32, CoreMask::Auto => NpuCoreMask::Unspecified as i32 }
}

fn plugin_request_timeout(plugin: &PluginDefinition) -> Duration {
    Duration::from_millis(plugin.summary.request_timeout_ms.clamp(1, 300_000))
}

fn inference_error(error: Status) -> SupervisorError {
    if error.code() == Code::InvalidArgument {
        SupervisorError::InvalidInput(error.message().to_owned())
    } else {
        SupervisorError::Inference(error.to_string())
    }
}

pub(crate) fn absolute_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() { path } else { std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(path) }
}

fn unix_time_micros() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_micros()
}
