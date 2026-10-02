use std::{
    collections::{BTreeMap, HashMap},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Extension, Json, Router,
    body::{Body, Bytes},
    extract::{ConnectInfo, DefaultBodyLimit, Path as AxumPath, Query, State},
    http::{HeaderMap, HeaderValue, Method, Request, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, delete, get, post, put},
};
use http_body::Body as HttpBody;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, RwLock};
use tower_http::trace::TraceLayer;

use crate::{
    auth::{
        ApiPrincipal, AuthRegistry, KeyRole, SCOPE_AUDIT, SCOPE_CONTROL, SCOPE_INFER, SCOPE_READ,
        TrustedProxies,
    },
    domain::{Allocation, CoreMask, PluginState, PluginSummary, validate_field_value},
    event_log::{AuditFields, EventPage, EventQuery, EventSeverity, EventStore},
    jobs::{JobError, JobManager, JobRequest, JobSnapshot},
    registry::{PluginDefinition, discover_plugins},
    scheduler::{ScheduleError, Scheduler},
    state_store::StateStore,
    supervisor::{
        Supervisor, SupervisorError, WorkerSnapshot, WorkerStatus, absolute_path,
    },
    telemetry::{read_device_telemetry, read_topology},
};

#[derive(Clone)]
pub struct AppState {
    plugins: Arc<Vec<PluginDefinition>>,
    scheduler: Arc<RwLock<Scheduler>>,
    supervisor: Supervisor,
    events: EventStore,
    state_store: StateStore,
    jobs: JobManager,
    control: Arc<Mutex<()>>,
    auth: AuthRegistry,
    trusted_proxies: TrustedProxies,
}

impl AppState {
    pub async fn load(
        plugin_root: &Path,
        runtime_root: PathBuf,
        state_root: PathBuf,
    ) -> anyhow::Result<Self> {
        let runtime_root = absolute_path(runtime_root);
        let state_root = absolute_path(state_root);
        let auth = AuthRegistry::from_env()?;
        let trusted_proxies = TrustedProxies::from_env()?;
        let events = EventStore::open(state_root.clone()).await?;
        let state_store = StateStore::open(state_root.join("state.json"))?;
        let scheduler = Arc::new(RwLock::new(Scheduler::default()));
        let plugins = Arc::new(discover_plugins(plugin_root)?);
        let supervisor = Supervisor::new(
            runtime_root,
            scheduler.clone(),
            events.clone(),
            state_store.clone(),
        );
        let jobs = JobManager::open(
            state_root.join("jobs"),
            supervisor.clone(),
            events.clone(),
        )
        .await?;
        let state = Self {
            plugins,
            supervisor,
            scheduler,
            events,
            state_store,
            jobs,
            control: Arc::new(Mutex::new(())),
            auth,
            trusted_proxies,
        };
        state.supervisor.reconcile_runtime().await;
        state.restore_desired_plugins().await;
        state
            .events
            .record_best_effort(
                EventSeverity::Info,
                "system",
                "core_started",
                "RKServe core started",
                None,
                None,
                None,
                None,
                None,
                Some("core".into()),
            )
            .await;
        Ok(state)
    }

    pub async fn shutdown(&self) {
        self
            .events
            .record_best_effort(
                EventSeverity::Info,
                "system",
                "core_stopping",
                "RKServe core is stopping",
                None,
                None,
                None,
                None,
                None,
                Some("core".into()),
            )
            .await;
        self.jobs.cancel_all().await;
        self.supervisor.shutdown_all().await;
    }

    pub fn api_key_count(&self) -> usize {
        self.auth.key_count()
    }

    pub fn trusts_forwarded_headers(&self) -> bool {
        !self.trusted_proxies.is_empty()
    }

    async fn restore_desired_plugins(&self) {
        for desired in self
            .state_store
            .desired()
            .into_iter()
            .filter(|item| item.enabled)
        {
            let Some(plugin) = self
                .plugins
                .iter()
                .find(|plugin| plugin.summary.id == desired.plugin_id)
            else {
                self
                    .events
                    .record_best_effort(
                        EventSeverity::Error,
                        "system",
                        "restore_missing_plugin",
                        "Cannot restore plugin: plugin is not installed",
                        None,
                        Some(desired.plugin_id),
                        None,
                        None,
                        None,
                        Some("core".into()),
                    )
                    .await;
                continue;
            };
            let allocation = match self
                .scheduler
                .write()
                .await
                .allocate(&plugin.summary, desired.core_mask)
            {
                Ok(allocation) => allocation,
                Err(error) => {
                    self
                        .events
                        .record_best_effort(
                            EventSeverity::Error,
                            "scheduler",
                            "restore_failed",
                            "Cannot restore the plugin NPU lease",
                            Some(serde_json::json!({"error": error.to_string()})),
                            Some(desired.plugin_id),
                            None,
                            None,
                            None,
                            Some("core".into()),
                        )
                        .await;
                    continue;
                }
            };
            if let Err(error) = self.supervisor.start(plugin, &allocation).await {
                let _ = self
                    .scheduler
                    .write()
                    .await
                    .release(&allocation.lease_id);
                let _ = self
                    .state_store
                    .fail(&desired.plugin_id, desired.core_mask, error.to_string())
                    .await;
            } else {
                self
                    .events
                    .record_best_effort(
                        EventSeverity::Info,
                        "system",
                        "plugin_restored",
                        "Desired plugin state restored",
                        None,
                        Some(desired.plugin_id),
                        Some(allocation.lease_id),
                        None,
                        None,
                        Some("core".into()),
                    )
                    .await;
            }
        }
    }
}

pub fn router(state: AppState) -> Router {
    let layer_state = ApiLayerState {
        auth: state.auth.clone(),
        trusted_proxies: state.trusted_proxies.clone(),
        events: state.events.clone(),
    };
    let protected = Router::new()
        .route("/api/v1/auth/whoami", get(whoami))
        .route("/api/v1/system", get(system))
        .route("/api/v1/system/telemetry", get(device_telemetry))
        .route("/api/v1/npu/topology", get(topology))
        .route("/api/v1/plugins", get(plugins))
        .route("/api/v1/plugins/{plugin_id}", get(plugin))
        .route("/api/v1/plugins/{plugin_id}/spec", get(plugin_spec))
        .route("/api/v1/workers", get(workers))
        .route("/api/v1/events", get(events))
        // POST uses {capability_id} to submit a new job under a capability.
        // GET and DELETE use {job_id} to query or cancel an existing job.
        // Both share the same URL slot; semantics differ by HTTP method.
        .route(
            "/api/v1/plugins/{plugin_id}/jobs/{job_or_capability_id}",
            post(submit_plugin_job),
        )
        .route(
            "/api/v1/plugins/{plugin_id}/jobs/{job_or_capability_id}",
            get(job_status).delete(cancel_job),
        )
        .route(
            "/api/v1/plugins/{plugin_id}/jobs/{job_id}/result",
            get(job_result),
        )
        .route(
            "/api/v1/plugins/{plugin_id}/configuration",
            put(update_plugin_configuration),
        )
        .route(
            "/api/v1/scheduler/allocations",
            get(allocations).post(create_allocation),
        )
        .route(
            "/api/v1/scheduler/allocations/{lease_id}",
            delete(delete_allocation),
        )
        .route("/api/{*path}", any(api_not_found))
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(layer_state, authenticate_and_audit));

    Router::new()
        .route("/health", get(health))
        .merge(protected)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[derive(Clone)]
struct ApiLayerState {
    auth: AuthRegistry,
    trusted_proxies: TrustedProxies,
    events: EventStore,
}

#[derive(Debug, Clone)]
struct ApiRequestContext {
    request_id: String,
    principal: ApiPrincipal,
}

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

async fn authenticate_and_audit(
    State(layer): State<ApiLayerState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let started = Instant::now();
    let method = request.method().clone();
    let uri = request.uri().clone();
    let path = uri.path().to_owned();
    let request_id = request_id(request.headers());
    let peer_ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(address)| address.ip());
    let client_ip = resolve_client_ip(peer_ip, request.headers(), &layer.trusted_proxies);
    let request_bytes = content_length(request.headers()).or_else(|| request.body().size_hint().exact());
    let request_content_type = media_type(request.headers());
    let user_agent = safe_header(request.headers(), header::USER_AGENT, 256);
    let required_scope = required_scope(&method, &path);
    let query_keys = query_keys(&uri);
    let (plugin_id, job_id) = resource_ids(&method, &path);

    let principal = api_token(request.headers())
        .and_then(|token| layer.auth.authenticate(token));
    let denied = match &principal {
        None => Some((
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "A valid API key is required (Authorization: Bearer or X-API-Key)",
        )),
        Some(principal) if !principal.allows_path(&path) => Some((
            StatusCode::FORBIDDEN,
            "role_denied",
            "API keys can only access inference job endpoints; use a management key for the console",
        )),
        Some(principal)
            if required_scope.is_some_and(|scope| !principal.allows(scope)) =>
        {
            Some((
                StatusCode::FORBIDDEN,
                "scope_denied",
                "The API key does not have permission to perform this operation",
            ))
        }
        Some(_) => None,
    };

    if let Some((status, kind, message)) = denied {
        let mut response = (
            status,
            Json(serde_json::json!({
                "error": message,
                "kind": kind,
                "request_id": request_id,
            })),
        )
            .into_response();
        if status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Bearer realm=\"rkserve\""),
            );
        }
        add_security_headers(&mut response, &request_id);
        let duration_us = started.elapsed().as_micros() as u64;
        log_api_request(&method, &path, status, duration_us, principal.as_ref(), &request_id);
        let mut metadata = audit_metadata(
            &method,
            &path,
            &query_keys,
            required_scope,
            request_content_type,
            response.headers(),
            user_agent,
        );
        if let Some(object) = metadata.as_mut().and_then(serde_json::Value::as_object_mut) {
            object.insert("error".into(), message.into());
        }
        persist_api_audit(
            &layer.events,
            status,
            kind,
            &audit_message(&method, &path, status, duration_us),
            metadata,
            plugin_id,
            job_id,
            request_id,
            principal.as_ref(),
            client_ip,
            peer_ip,
            duration_us,
            request_bytes,
            response_bytes(&response),
        )
        .await;
        return response;
    }

    let principal = principal.expect("denied unauthenticated request above");
    request.extensions_mut().insert(ApiRequestContext {
        request_id: request_id.clone(),
        principal: principal.clone(),
    });
    let mut response = next.run(request).await;
    add_security_headers(&mut response, &request_id);
    let status = response.status();
    let duration_us = started.elapsed().as_micros() as u64;
    log_api_request(&method, &path, status, duration_us, Some(&principal), &request_id);
    if should_persist_api_event(&method, &path, status) {
        let metadata = audit_metadata(
            &method,
            &path,
            &query_keys,
            required_scope,
            request_content_type,
            response.headers(),
            user_agent,
        );
        persist_api_audit(
            &layer.events,
            status,
            if status.is_success() {
                "request_completed"
            } else {
                "request_failed"
            },
            &audit_message(&method, &path, status, duration_us),
            metadata,
            plugin_id,
            job_id,
            request_id,
            Some(&principal),
            client_ip,
            peer_ip,
            duration_us,
            request_bytes,
            response_bytes(&response),
        )
        .await;
    }
    response
}

#[derive(Serialize)]
struct WhoAmIResponse {
    name: String,
    role: KeyRole,
    scopes: std::collections::BTreeSet<String>,
    fingerprint: String,
}

async fn whoami(Extension(context): Extension<ApiRequestContext>) -> Json<WhoAmIResponse> {
    Json(WhoAmIResponse {
        name: context.principal.name,
        role: context.principal.role,
        scopes: context.principal.scopes,
        fingerprint: context.principal.fingerprint,
    })
}

fn required_scope(method: &Method, path: &str) -> Option<&'static str> {
    if path == "/api/v1/auth/whoami" {
        None
    } else if method == Method::GET && path == "/api/v1/events" {
        Some(SCOPE_AUDIT)
    } else if path.contains("/jobs/") {
        Some(SCOPE_INFER)
    } else if matches!(*method, Method::POST | Method::PUT | Method::PATCH | Method::DELETE) {
        Some(SCOPE_CONTROL)
    } else {
        Some(SCOPE_READ)
    }
}

fn api_token(headers: &HeaderMap) -> Option<&str> {
    bearer_token(headers).or_else(|| header_secret(headers, "x-api-key"))
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    usable_secret(token)
}

fn header_secret<'a>(headers: &'a HeaderMap, name: &'static str) -> Option<&'a str> {
    usable_secret(headers.get(name)?.to_str().ok()?)
}

fn usable_secret(value: &str) -> Option<&str> {
    let token = value.trim();
    if token.len() < 32 || token.chars().any(char::is_whitespace) {
        return None;
    }
    Some(token)
}

fn audit_message(method: &Method, path: &str, status: StatusCode, duration_us: u64) -> String {
    format!(
        "{} {} → {} · {:.1} ms",
        method.as_str(),
        path,
        status.as_u16(),
        duration_us as f64 / 1000.0
    )
}

fn log_api_request(
    method: &Method,
    path: &str,
    status: StatusCode,
    duration_us: u64,
    principal: Option<&ApiPrincipal>,
    request_id: &str,
) {
    let actor = principal.map(|item| item.name.as_str()).unwrap_or("anonymous");
    let duration_ms = duration_us as f64 / 1000.0;
    if status.is_server_error() {
        tracing::error!(%request_id, actor, method = %method, path, status = status.as_u16(), duration_ms, "api request");
    } else if status.is_client_error() {
        tracing::warn!(%request_id, actor, method = %method, path, status = status.as_u16(), duration_ms, "api request");
    } else if is_poll_get(method, path) {
        tracing::debug!(%request_id, actor, method = %method, path, status = status.as_u16(), duration_ms, "api request");
    } else {
        tracing::info!(%request_id, actor, method = %method, path, status = status.as_u16(), duration_ms, "api request");
    }
}

fn is_poll_get(method: &Method, path: &str) -> bool {
    matches!(*method, Method::GET | Method::HEAD)
        && matches!(
            path,
            "/api/v1/npu/topology"
                | "/api/v1/system/telemetry"
                | "/api/v1/workers"
                | "/api/v1/scheduler/allocations"
                | "/api/v1/plugins"
                | "/api/v1/events"
        )
}

fn request_id(headers: &HeaderMap) -> String {
    if let Some(value) = headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 64
                && value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
                })
        })
    {
        return value.to_owned();
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let sequence = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("req-{timestamp:x}-{sequence:x}")
}

fn resolve_client_ip(
    peer_ip: Option<IpAddr>,
    headers: &HeaderMap,
    trusted_proxies: &TrustedProxies,
) -> Option<IpAddr> {
    let peer = peer_ip?;
    if !trusted_proxies.contains(&peer) {
        return Some(peer);
    }
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .and_then(|value| value.parse().ok())
        .or(Some(peer))
}

fn content_length(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
}

fn response_bytes(response: &Response) -> Option<u64> {
    content_length(response.headers()).or_else(|| response.body().size_hint().exact())
}

fn media_type(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn safe_header(headers: &HeaderMap, name: header::HeaderName, limit: usize) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .chars()
                .filter(|character| !character.is_control())
                .take(limit)
                .collect()
        })
}

fn query_keys(uri: &Uri) -> Vec<String> {
    let mut keys = uri
        .query()
        .into_iter()
        .flat_map(|query| query.split('&'))
        .filter_map(|pair| pair.split_once('=').map_or(Some(pair), |(key, _)| Some(key)))
        .filter(|key| !key.is_empty())
        .map(|key| key.chars().take(64).collect::<String>())
        .collect::<Vec<_>>();
    keys.sort();
    keys.dedup();
    keys
}

fn resource_ids(method: &Method, path: &str) -> (Option<String>, Option<String>) {
    let segments = path.trim_matches('/').split('/').collect::<Vec<_>>();
    let plugin_id = segments
        .windows(2)
        .find(|pair| pair[0] == "plugins")
        .map(|pair| pair[1].to_owned());
    let job_id = segments
        .windows(2)
        .find(|pair| pair[0] == "jobs")
        .and_then(|pair| (method != Method::POST).then(|| pair[1].to_owned()));
    (plugin_id, job_id)
}

fn audit_metadata(
    method: &Method,
    path: &str,
    query_keys: &[String],
    required_scope: Option<&str>,
    request_content_type: Option<String>,
    response_headers: &HeaderMap,
    user_agent: Option<String>,
) -> Option<serde_json::Value> {
    Some(serde_json::json!({
        "method": method.as_str(),
        "path": path,
        "query_keys": query_keys,
        "required_scope": required_scope,
        "request_content_type": request_content_type,
        "response_content_type": media_type(response_headers),
        "user_agent": user_agent,
    }))
}

#[allow(clippy::too_many_arguments)]
async fn persist_api_audit(
    events: &EventStore,
    status: StatusCode,
    kind: &str,
    message: &str,
    mut metadata: Option<serde_json::Value>,
    plugin_id: Option<String>,
    job_id: Option<String>,
    request_id: String,
    principal: Option<&ApiPrincipal>,
    client_ip: Option<IpAddr>,
    peer_ip: Option<IpAddr>,
    duration_us: u64,
    request_bytes: Option<u64>,
    response_bytes: Option<u64>,
) {
    let severity = if status.is_server_error() {
        EventSeverity::Error
    } else if status.is_client_error() {
        EventSeverity::Warning
    } else {
        EventSeverity::Info
    };
    let category = if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        "auth"
    } else {
        "api"
    };
    if let (Some(principal), Some(object)) = (
        principal,
        metadata
            .as_mut()
            .and_then(serde_json::Value::as_object_mut),
    ) {
        object.insert(
            "key_fingerprint".into(),
            principal.fingerprint.clone().into(),
        );
        object.insert("key_role".into(), principal.role.as_str().into());
        object.insert(
            "granted_scopes".into(),
            serde_json::to_value(&principal.scopes).unwrap_or_default(),
        );
    }
    let http_method = metadata
        .as_ref()
        .and_then(|value| value.get("method"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let http_path = metadata
        .as_ref()
        .and_then(|value| value.get("path"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    events
        .record_audit_best_effort(
            severity,
            category,
            kind,
            message,
            metadata,
            plugin_id,
            None,
            job_id,
            Some(request_id),
            Some("http".into()),
            AuditFields {
                actor: principal.map(|principal| principal.name.clone()),
                client_ip: client_ip.map(|address| address.to_string()),
                peer_ip: peer_ip.map(|address| address.to_string()),
                http_method,
                http_path,
                http_status: Some(status.as_u16()),
                duration_us: Some(duration_us),
                request_bytes,
                response_bytes,
            },
        )
        .await;
}

fn should_persist_api_event(method: &Method, path: &str, status: StatusCode) -> bool {
    !status.is_success() || !is_poll_get(method, path)
}

fn add_security_headers(response: &mut Response, request_id: &str) {
    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(request_id).expect("validated request ID"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn api_not_found() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "API route not found" })),
    )
}

#[derive(Serialize)]
struct SystemResponse {
    name: &'static str,
    version: &'static str,
    platform: &'static str,
    plugin_count: usize,
}

async fn system(State(state): State<AppState>) -> Json<SystemResponse> {
    Json(SystemResponse {
        name: "rkserve",
        version: env!("CARGO_PKG_VERSION"),
        platform: crate::PLATFORM,
        plugin_count: state.plugins.len(),
    })
}

async fn device_telemetry() -> Json<crate::domain::DeviceTelemetry> {
    Json(read_device_telemetry())
}

async fn topology(State(state): State<AppState>) -> Json<crate::domain::NpuTopology> {
    let scheduler = state.scheduler.read().await;
    let mut topology = read_topology(scheduler.allocations());
    topology.runtime_version = state
        .supervisor
        .snapshots()
        .await
        .first()
        .map(|worker| worker.runtime_version.clone());
    Json(topology)
}

async fn plugins(State(state): State<AppState>) -> Json<Vec<PluginSummary>> {
    Json(plugin_summaries(&state).await)
}

async fn plugin(
    State(state): State<AppState>,
    AxumPath(plugin_id): AxumPath<String>,
) -> Result<Json<PluginSummary>, ApiError> {
    let plugin = plugin_summaries(&state)
        .await
        .into_iter()
        .find(|plugin| plugin.id == plugin_id)
        .ok_or_else(|| ScheduleError::PluginNotFound(plugin_id))?;
    Ok(Json(plugin))
}

async fn plugin_summaries(state: &AppState) -> Vec<PluginSummary> {
    let running = state.supervisor.snapshots().await;
    state
        .plugins
        .iter()
        .map(|definition| {
            let mut summary = definition.summary.clone();
            let desired = state.state_store.get(&summary.id);
            let stored = desired
                .as_ref()
                .map(|desired| desired.configuration.clone())
                .unwrap_or_default();
            summary.configuration_values =
                definition.resolve_configuration(&stored).unwrap_or_default();
            if let Some(worker) = running
                .iter()
                .find(|worker| worker.plugin_id == summary.id)
            {
                summary.state = match worker.status {
                    WorkerStatus::Ready => PluginState::Ready,
                    WorkerStatus::Backoff => PluginState::Backoff,
                };
            } else if desired.is_some_and(|desired| desired.last_error.is_some()) {
                summary.state = PluginState::Failed;
            }
            summary
        })
        .collect()
}

async fn workers(State(state): State<AppState>) -> Json<Vec<WorkerSnapshot>> {
    Json(state.supervisor.snapshots().await)
}

async fn allocations(State(state): State<AppState>) -> Json<Vec<Allocation>> {
    Json(state.scheduler.read().await.allocations().to_vec())
}

#[derive(Serialize)]
struct PluginSpecResponse {
    plugin: PluginSummary,
    jobs: JobApiDocumentation,
}

#[derive(Serialize)]
struct JobApiDocumentation {
    submit: ApiOperation,
    get: ApiOperation,
    cancel: ApiOperation,
    result: ApiOperation,
    capabilities: Vec<crate::domain::CapabilityDefinition>,
}

#[derive(Serialize)]
struct ApiOperation {
    method: &'static str,
    path: String,
    description: &'static str,
}

async fn plugin_spec(
    State(state): State<AppState>,
    AxumPath(plugin_id): AxumPath<String>,
) -> Result<Json<PluginSpecResponse>, ApiError> {
    let summary = plugin_summaries(&state)
        .await
        .into_iter()
        .find(|plugin| plugin.id == plugin_id)
        .ok_or_else(|| ScheduleError::PluginNotFound(plugin_id.clone()))?;
    let base = format!("/api/v1/plugins/{plugin_id}/jobs");
    Ok(Json(PluginSpecResponse {
        jobs: JobApiDocumentation {
            submit: ApiOperation {
                method: "POST",
                path: format!("{base}/{{capability_id}}"),
                description: "Create an asynchronous job with the capability input payload and query parameters.",
            },
            get: ApiOperation {
                method: "GET",
                path: format!("{base}/{{job_id}}"),
                description: "Get this plugin's job status.",
            },
            cancel: ApiOperation {
                method: "DELETE",
                path: format!("{base}/{{job_id}}"),
                description: "Request cancellation of this plugin's job.",
            },
            result: ApiOperation {
                method: "GET",
                path: format!("{base}/{{job_id}}/result"),
                description: "Get a completed job's binary result.",
            },
            capabilities: summary.capabilities.clone(),
        },
        plugin: summary,
    }))
}

#[derive(Debug, Deserialize)]
struct EventsQuery {
    after: Option<u64>,
    before: Option<u64>,
    cursor: Option<u64>,
    severity: Option<EventSeverity>,
    category: Option<String>,
    kind: Option<String>,
    plugin_id: Option<String>,
    lease_id: Option<String>,
    job_id: Option<String>,
    request_id: Option<String>,
    actor: Option<String>,
    client_ip: Option<String>,
    http_method: Option<String>,
    http_status: Option<u16>,
    source: Option<String>,
    exclude_source: Option<String>,
    q: Option<String>,
    outcome: Option<String>,
    path_prefix: Option<String>,
    from: Option<u64>,
    to: Option<u64>,
    #[serde(default = "default_event_limit")]
    limit: usize,
}

fn default_event_limit() -> usize {
    100
}

async fn events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<EventPage>, ApiError> {
    if query.before.is_some() && query.cursor.is_some() {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "before and cursor cannot be combined".into(),
        });
    }
    if query.q.as_ref().is_some_and(|value| value.chars().count() > 64) {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "q must be at most 64 characters".into(),
        });
    }
    if query.path_prefix.as_ref().is_some_and(|value| value.chars().count() > 128) {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "path_prefix must be at most 128 characters".into(),
        });
    }
    if let Some(outcome) = query.outcome.as_deref() {
        if !matches!(outcome, "success" | "failure") {
            return Err(ApiError {
                status: StatusCode::BAD_REQUEST,
                message: "outcome must be success or failure".into(),
            });
        }
    }
    Ok(Json(
        state
            .events
            .query(EventQuery {
                cursor: query.before.or(query.cursor),
                after: query.after,
                severity: query.severity,
                category: query.category,
                kind: query.kind,
                plugin_id: query.plugin_id,
                lease_id: query.lease_id,
                job_id: query.job_id,
                request_id: query.request_id,
                actor: query.actor,
                client_ip: query.client_ip,
                http_method: query.http_method.map(|method| method.to_ascii_uppercase()),
                http_status: query.http_status,
                source: query.source,
                exclude_source: query.exclude_source,
                q: query.q,
                outcome: query.outcome,
                path_prefix: query.path_prefix,
                from_unix_ms: query.from,
                to_unix_ms: query.to,
                limit: query.limit,
            })
            .await
            .map_err(ApiError::internal)?,
    ))
}

async fn submit_plugin_job(
    State(state): State<AppState>,
    Extension(context): Extension<ApiRequestContext>,
    AxumPath((plugin_id, capability_id)): AxumPath<(String, String)>,
    Query(parameters): Query<HashMap<String, String>>,
    headers: HeaderMap,
    payload: Bytes,
) -> Result<(StatusCode, Json<JobSnapshot>), ApiError> {
    let plugin = find_plugin(&state, &plugin_id)?;
    let capability = plugin
        .summary
        .capabilities
        .iter()
        .find(|capability| capability.id == capability_id)
        .ok_or_else(|| {
            ApiError::not_found(format!("capability '{capability_id}' was not found"))
        })?;
    let content_type = validate_invocation(capability, &parameters, &headers, payload.len())?;
    let input_sha256 = Sha256::digest(&payload)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let snapshot = state
        .jobs
        .submit(JobRequest {
            plugin_id: plugin.summary.id.clone(),
            capability_id: capability.id.clone(),
            content_type,
            expected_output_content_type: capability.output_content_type.clone(),
            payload: payload.to_vec(),
            parameters,
            max_active: (plugin.summary.queue_size + plugin.summary.max_concurrency) as usize,
            max_concurrency: plugin.summary.max_concurrency as usize,
            request_id: context.request_id,
            submitted_by: context.principal.name,
            input_sha256,
        })
        .await?;
    Ok((StatusCode::ACCEPTED, Json(snapshot)))
}

async fn job_status(
    State(state): State<AppState>,
    AxumPath((plugin_id, job_id)): AxumPath<(String, String)>,
) -> Result<Json<JobSnapshot>, ApiError> {
    Ok(Json(state.jobs.get(&plugin_id, &job_id).await?))
}

async fn cancel_job(
    State(state): State<AppState>,
    AxumPath((plugin_id, job_id)): AxumPath<(String, String)>,
) -> Result<(StatusCode, Json<JobSnapshot>), ApiError> {
    Ok((
        StatusCode::ACCEPTED,
        Json(state.jobs.cancel(&plugin_id, &job_id).await?),
    ))
}

async fn job_result(
    State(state): State<AppState>,
    AxumPath((plugin_id, job_id)): AxumPath<(String, String)>,
) -> Result<(HeaderMap, Bytes), ApiError> {
    let result = state.jobs.result(&plugin_id, &job_id).await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(
            result
                .snapshot
                .content_type
                .as_deref()
                .unwrap_or("application/octet-stream"),
        )
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    if let Some(timings) = result.snapshot.timings {
        for (name, value) in [
            ("x-rkserve-queue-us", timings.queue_us),
            ("x-rkserve-preprocess-us", timings.preprocess_us),
            ("x-rkserve-inference-us", timings.inference_us),
            ("x-rkserve-postprocess-us", timings.postprocess_us),
        ] {
            headers.insert(name, HeaderValue::from_str(&value.to_string()).unwrap());
        }
    }
    Ok((headers, Bytes::from(result.payload)))
}

fn find_plugin<'a>(
    state: &'a AppState,
    plugin_id: &str,
) -> Result<&'a PluginDefinition, ApiError> {
    state
        .plugins
        .iter()
        .find(|plugin| plugin.summary.id == plugin_id)
        .ok_or_else(|| ScheduleError::PluginNotFound(plugin_id.into()).into())
}

fn validate_invocation(
    capability: &crate::domain::CapabilityDefinition,
    parameters: &HashMap<String, String>,
    headers: &HeaderMap,
    payload_size: usize,
) -> Result<String, ApiError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .split(';')
        .next()
        .unwrap_or("application/octet-stream")
        .trim()
        .to_owned();
    if !capability
        .accepted_content_types
        .iter()
        .any(|accepted| accepted == &content_type)
    {
        return Err(ApiError {
            status: StatusCode::UNSUPPORTED_MEDIA_TYPE,
            message: format!(
                "expected one of: {}",
                capability.accepted_content_types.join(", ")
            ),
        });
    }
    if payload_size > capability.max_input_bytes {
        return Err(ApiError {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            message: format!("payload exceeds {} bytes", capability.max_input_bytes),
        });
    }
    for (key, value) in parameters {
        let field = capability
            .parameters
            .iter()
            .find(|field| &field.key == key)
            .ok_or_else(|| ApiError {
                status: StatusCode::BAD_REQUEST,
                message: format!("unknown capability parameter '{key}'"),
            })?;
        validate_capability_parameter(field, value)?;
    }
    for field in capability.parameters.iter().filter(|field| field.required) {
        if !parameters
            .get(&field.key)
            .is_some_and(|value| !value.trim().is_empty())
            && field.default.is_none()
        {
            return Err(ApiError {
                status: StatusCode::BAD_REQUEST,
                message: format!("missing required capability parameter '{}'", field.key),
            });
        }
    }
    Ok(content_type)
}

fn validate_capability_parameter(
    field: &crate::domain::ConfigurationField,
    value: &str,
) -> Result<(), ApiError> {
    validate_field_value(field, value).map_err(|reason| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: format!("parameter '{}' {reason}", field.key),
    })
}

#[derive(Debug, Deserialize)]
struct ConfigurationRequest {
    values: BTreeMap<String, String>,
}

async fn update_plugin_configuration(
    State(state): State<AppState>,
    AxumPath(plugin_id): AxumPath<String>,
    Json(request): Json<ConfigurationRequest>,
) -> Result<Json<BTreeMap<String, String>>, ApiError> {
    let _control = state.control.lock().await;
    let plugin = find_plugin(&state, &plugin_id)?;
    let resolved = plugin
        .resolve_configuration(&request.values)
        .map_err(|error| ApiError {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: error.to_string(),
        })?;

    let previous_configuration = plugin
        .resolve_configuration(
            &state
                .state_store
                .get(&plugin_id)
                .map(|desired| desired.configuration)
                .unwrap_or_default(),
        )
        .map_err(ApiError::internal)?;
    let allocation = state
        .scheduler
        .read()
        .await
        .allocations()
        .iter()
        .find(|allocation| allocation.plugin_id == plugin_id)
        .cloned();

    if resolved == previous_configuration {
        return Ok(Json(resolved));
    }

    if let Some(allocation) = &allocation {
        state.supervisor.stop(&allocation.lease_id).await?;
    }

    if let Err(error) = state
        .state_store
        .set_configuration(
            &plugin_id,
            allocation
                .as_ref()
                .map_or(plugin.summary.default_mask, |allocation| allocation.core_mask),
            resolved.clone(),
        )
        .await
    {
        let rollback = match &allocation {
            Some(previous_allocation) => {
                restart_plugin(&state, plugin, previous_allocation.core_mask).await
            }
            None => Ok(()),
        };
        return Err(configuration_switch_error(error, rollback));
    }

    if let Some(previous_allocation) = &allocation {
        let next_allocation = match state
            .scheduler
            .write()
            .await
            .allocate(&plugin.summary, previous_allocation.core_mask)
        {
            Ok(allocation) => allocation,
            Err(error) => {
                let rollback = rollback_plugin_configuration(
                    &state,
                    plugin,
                    previous_allocation.core_mask,
                    previous_configuration,
                )
                .await;
                return Err(configuration_switch_error(error, rollback));
            }
        };
        if let Err(error) = state.supervisor.start(plugin, &next_allocation).await {
            let _ = state
                .scheduler
                .write()
                .await
                .release(&next_allocation.lease_id);
            let rollback = rollback_plugin_configuration(
                &state,
                plugin,
                previous_allocation.core_mask,
                previous_configuration,
            )
            .await;
            return Err(configuration_switch_error(error, rollback));
        }
    }

    state
        .events
        .record_best_effort(
            EventSeverity::Info,
            "plugin",
            "configuration_updated",
            "Plugin configuration updated",
            Some(serde_json::json!({
                "worker_reloaded": allocation.is_some()
            })),
            Some(plugin_id),
            None,
            None,
            None,
            Some("core".into()),
        )
        .await;
    Ok(Json(resolved))
}

async fn rollback_plugin_configuration(
    state: &AppState,
    plugin: &PluginDefinition,
    core_mask: CoreMask,
    configuration: BTreeMap<String, String>,
) -> Result<(), String> {
    state
        .state_store
        .set_configuration(&plugin.summary.id, core_mask, configuration)
        .await
        .map_err(|error| format!("restore previous configuration: {error}"))?;
    restart_plugin(state, plugin, core_mask).await
}

async fn restart_plugin(
    state: &AppState,
    plugin: &PluginDefinition,
    core_mask: CoreMask,
) -> Result<(), String> {
    let allocation = state
        .scheduler
        .write()
        .await
        .allocate(&plugin.summary, core_mask)
        .map_err(|error| format!("restore NPU lease: {error}"))?;
    if let Err(error) = state.supervisor.start(plugin, &allocation).await {
        let _ = state
            .scheduler
            .write()
            .await
            .release(&allocation.lease_id);
        return Err(format!("restart previous worker: {error}"));
    }
    Ok(())
}

fn configuration_switch_error(
    error: impl std::fmt::Display,
    rollback: Result<(), String>,
) -> ApiError {
    let rollback_detail = match rollback {
        Ok(()) => "previous configuration was restored".to_owned(),
        Err(rollback_error) => format!("rollback also failed: {rollback_error}"),
    };
    ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: format!("could not reload plugin with the new configuration: {error}; {rollback_detail}"),
    }
}

#[derive(Debug, Deserialize)]
struct AllocateRequest {
    plugin_id: String,
    #[serde(default = "auto_mask")]
    core_mask: CoreMask,
}

fn auto_mask() -> CoreMask {
    CoreMask::Auto
}

async fn create_allocation(
    State(state): State<AppState>,
    Json(request): Json<AllocateRequest>,
) -> Result<(StatusCode, Json<Allocation>), ApiError> {
    let _control = state.control.lock().await;
    let plugin = find_plugin(&state, &request.plugin_id)?;
    let allocation = state
        .scheduler
        .write()
        .await
        .allocate(&plugin.summary, request.core_mask)?;
    if let Err(error) = state.supervisor.start(plugin, &allocation).await {
        let _ = state
            .scheduler
            .write()
            .await
            .release(&allocation.lease_id);
        return Err(error.into());
    }
    if let Err(error) = state
        .state_store
        .enable(&plugin.summary.id, allocation.core_mask)
        .await
    {
        if let Err(stop_error) = state.supervisor.stop(&allocation.lease_id).await {
            state
                .events
                .record_best_effort(
                    EventSeverity::Error,
                    "scheduler",
                    "lease_rollback_failed",
                    "Failed to persist lease state and roll back the worker",
                    Some(serde_json::json!({"error": stop_error.to_string()})),
                    Some(plugin.summary.id.clone()),
                    Some(allocation.lease_id.clone()),
                    None,
                    None,
                    Some("core".into()),
                )
                .await;
        }
        return Err(ApiError::internal(error));
    }
    state
        .events
        .record_best_effort(
            EventSeverity::Info,
            "scheduler",
            "lease_created",
            "NPU lease created",
            Some(serde_json::json!({
                "core_mask": allocation.core_mask.label()
            })),
            Some(plugin.summary.id.clone()),
            Some(allocation.lease_id.clone()),
            None,
            None,
            Some("core".into()),
        )
        .await;
    Ok((StatusCode::CREATED, Json(allocation)))
}

async fn delete_allocation(
    State(state): State<AppState>,
    AxumPath(lease_id): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    let _control = state.control.lock().await;
    let allocation = state
        .scheduler
        .read()
        .await
        .allocations()
        .iter()
        .find(|allocation| allocation.lease_id == lease_id)
        .cloned()
        .ok_or_else(|| ScheduleError::LeaseNotFound(lease_id.clone()))?;
    state
        .state_store
        .disable(&allocation.plugin_id, allocation.core_mask)
        .await
        .map_err(ApiError::internal)?;
    if let Err(error) = state.supervisor.stop(&lease_id).await {
        if let Err(restore_error) = state
            .state_store
            .enable(&allocation.plugin_id, allocation.core_mask)
            .await
        {
            state
                .events
                .record_best_effort(
                    EventSeverity::Error,
                    "scheduler",
                    "lease_restore_failed",
                    "Failed to stop the worker and restore the desired lease state",
                    Some(serde_json::json!({
                        "error": restore_error.to_string()
                    })),
                    Some(allocation.plugin_id.clone()),
                    Some(allocation.lease_id.clone()),
                    None,
                    None,
                    Some("core".into()),
                )
                .await;
        }
        return Err(error.into());
    }
    state
        .events
        .record_best_effort(
            EventSeverity::Info,
            "scheduler",
            "lease_released",
            "NPU lease released",
            None,
            Some(allocation.plugin_id),
            Some(allocation.lease_id),
            None,
            None,
            Some("core".into()),
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn not_found(message: String) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message,
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl From<ScheduleError> for ApiError {
    fn from(value: ScheduleError) -> Self {
        let status = match value {
            ScheduleError::PluginNotFound(_) | ScheduleError::LeaseNotFound(_) => {
                StatusCode::NOT_FOUND
            }
            ScheduleError::MaskNotAllowed(_) => StatusCode::UNPROCESSABLE_ENTITY,
            ScheduleError::CoresBusy => StatusCode::CONFLICT,
        };
        Self {
            status,
            message: value.to_string(),
        }
    }
}

impl From<SupervisorError> for ApiError {
    fn from(value: SupervisorError) -> Self {
        let status = match value {
            SupervisorError::AlreadyRunning(_) => StatusCode::CONFLICT,
            SupervisorError::WorkerNotFound(_) => StatusCode::NOT_FOUND,
            SupervisorError::InvalidInput(_) => StatusCode::BAD_REQUEST,
            SupervisorError::IdentityMismatch
            | SupervisorError::ProtocolMismatch
            | SupervisorError::NpuRequired
            | SupervisorError::CoreMaskMismatch
            | SupervisorError::CapabilityMismatch
            | SupervisorError::ModelTargetMismatch => StatusCode::UNPROCESSABLE_ENTITY,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        };
        Self {
            status,
            message: value.to_string(),
        }
    }
}

impl From<JobError> for ApiError {
    fn from(value: JobError) -> Self {
        let status = match value {
            JobError::QueueFull => StatusCode::TOO_MANY_REQUESTS,
            JobError::NotFound(_) => StatusCode::NOT_FOUND,
            JobError::ResultNotReady | JobError::AlreadyComplete => StatusCode::CONFLICT,
            JobError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: value.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        headers
    }

    #[test]
    fn accepts_bearer_and_x_api_key() {
        let bearer = headers(&[("authorization", "Bearer 0123456789abcdef0123456789abcdef")]);
        assert_eq!(
            api_token(&bearer),
            Some("0123456789abcdef0123456789abcdef")
        );
        let x_key = headers(&[("x-api-key", "fedcba9876543210fedcba9876543210xxxx")]);
        assert_eq!(
            api_token(&x_key),
            Some("fedcba9876543210fedcba9876543210xxxx")
        );
        let both = headers(&[
            ("authorization", "Bearer 0123456789abcdef0123456789abcdef"),
            ("x-api-key", "fedcba9876543210fedcba9876543210xxxx"),
        ]);
        assert_eq!(
            api_token(&both),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert!(api_token(&headers(&[("x-api-key", "short")])).is_none());
    }

    #[test]
    fn skips_console_poll_gets_but_keeps_mutations_and_failures() {
        assert!(!should_persist_api_event(
            &Method::GET,
            "/api/v1/npu/topology",
            StatusCode::OK
        ));
        assert!(!should_persist_api_event(
            &Method::GET,
            "/api/v1/events",
            StatusCode::OK
        ));
        assert!(should_persist_api_event(
            &Method::GET,
            "/api/v1/npu/topology",
            StatusCode::UNAUTHORIZED
        ));
        assert!(should_persist_api_event(
            &Method::POST,
            "/api/v1/scheduler/allocations",
            StatusCode::CREATED
        ));
        assert!(should_persist_api_event(
            &Method::GET,
            "/api/v1/auth/whoami",
            StatusCode::OK
        ));
        assert!(should_persist_api_event(
            &Method::GET,
            "/api/v1/plugins/yolo26/spec",
            StatusCode::OK
        ));
    }
}

