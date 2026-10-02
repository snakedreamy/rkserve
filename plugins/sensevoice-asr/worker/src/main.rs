use std::{
    ffi::{CStr, CString, c_char, c_void},
    path::PathBuf,
    ptr::NonNull,
    slice,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use rkserve_protocol::{
    PROTOCOL_VERSION,
    plugin::v1::{
        DescribeRequest, DescribeResponse, DrainRequest, DrainResponse, ExecuteJobRequest,
        ExecuteJobResponse, HealthRequest, HealthResponse, LoadRequest, LoadResponse, UnloadRequest,
        UnloadResponse, health_response::ServingStatus,
        plugin_runtime_server::{PluginRuntime, PluginRuntimeServer},
    },
};
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::{Request, Response, Status, transport::Server};
use tracing::info;

const MAX_RPC_MESSAGE_SIZE: usize = 64 * 1024 * 1024;

enum NativeEngine {}

#[derive(Default)]
#[repr(C)]
struct NativeTiming {
    preprocess_us: u64,
    inference_us: u64,
    postprocess_us: u64,
    audio_duration_ms: u64,
    segment_count: u64,
}

unsafe extern "C" {
    fn rkserve_sensevoice_asr_create(
        model_path: *const c_char,
        cmvn_path: *const c_char,
        embedding_path: *const c_char,
        tokens_path: *const c_char,
        core_mask: i32,
        error: *mut c_char,
        error_capacity: usize,
    ) -> *mut NativeEngine;
    fn rkserve_sensevoice_asr_destroy(engine: *mut NativeEngine);
    fn rkserve_sensevoice_asr_transcribe(
        engine: *mut NativeEngine,
        wav: *const u8,
        wav_size: usize,
        language: *const c_char,
        with_itn: i32,
        json: *mut *mut c_char,
        json_size: *mut usize,
        timing: *mut NativeTiming,
        error: *mut c_char,
        error_capacity: usize,
    ) -> i32;
    fn rkserve_sensevoice_asr_free(pointer: *mut c_void);
    fn rkserve_sensevoice_asr_versions(
        engine: *mut NativeEngine,
        runtime: *mut c_char,
        runtime_capacity: usize,
        driver: *mut c_char,
        driver_capacity: usize,
    ) -> i32;
}

struct Transcription {
    json: Vec<u8>,
    timing: NativeTiming,
}

struct Engine {
    native: NonNull<NativeEngine>,
    runtime_version: String,
    driver_version: String,
}

unsafe impl Send for Engine {}

impl Engine {
    fn load(core_mask: i32) -> Result<Self, String> {
        let model = c_path(plugin_asset(
            "assets/models/sense-voice-encoder.rk3576.fp16.rknn",
        )?)?;
        let cmvn = c_path(plugin_asset("assets/text/am.mvn")?)?;
        let embedding = c_path(plugin_asset("assets/text/embedding.npy")?)?;
        let tokens = c_path(plugin_asset("assets/text/tokens.txt")?)?;
        let mut error = [0 as c_char; 1024];
        let native = unsafe {
            rkserve_sensevoice_asr_create(
                model.as_ptr(),
                cmvn.as_ptr(),
                embedding.as_ptr(),
                tokens.as_ptr(),
                core_mask,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        let native = NonNull::new(native).ok_or_else(|| read_c_string(&error))?;

        let mut runtime = [0 as c_char; 256];
        let mut driver = [0 as c_char; 128];
        if unsafe {
            rkserve_sensevoice_asr_versions(
                native.as_ptr(),
                runtime.as_mut_ptr(),
                runtime.len(),
                driver.as_mut_ptr(),
                driver.len(),
            )
        } != 0
        {
            unsafe { rkserve_sensevoice_asr_destroy(native.as_ptr()) };
            return Err("could not query RKNN runtime versions".into());
        }
        Ok(Self {
            native,
            runtime_version: read_c_string(&runtime),
            driver_version: read_c_string(&driver),
        })
    }

    fn transcribe(
        &mut self,
        wav: &[u8],
        language: &str,
        with_itn: bool,
    ) -> Result<Transcription, String> {
        let language = CString::new(language).map_err(|_| "language contains NUL".to_owned())?;
        let mut json = std::ptr::null_mut();
        let mut json_size = 0usize;
        let mut timing = NativeTiming::default();
        let mut error = [0 as c_char; 1024];
        let result = unsafe {
            rkserve_sensevoice_asr_transcribe(
                self.native.as_ptr(),
                wav.as_ptr(),
                wav.len(),
                language.as_ptr(),
                i32::from(with_itn),
                &mut json,
                &mut json_size,
                &mut timing,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if result != 0 {
            if !json.is_null() {
                unsafe { rkserve_sensevoice_asr_free(json.cast()) };
            }
            return Err(read_c_string(&error));
        }
        if json.is_null() && json_size != 0 {
            return Err("native engine returned an invalid JSON buffer".into());
        }
        let bytes = if json_size == 0 {
            Vec::new()
        } else {
            unsafe { slice::from_raw_parts(json.cast::<u8>(), json_size) }.to_vec()
        };
        if !json.is_null() {
            unsafe { rkserve_sensevoice_asr_free(json.cast()) };
        }
        std::str::from_utf8(&bytes)
            .map_err(|_| "native engine returned invalid UTF-8 JSON".to_owned())?;
        Ok(Transcription {
            json: bytes,
            timing,
        })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { rkserve_sensevoice_asr_destroy(self.native.as_ptr()) };
    }
}

#[derive(Clone, Default)]
struct SenseVoiceWorker {
    engine: Arc<Mutex<Option<Engine>>>,
    draining: Arc<AtomicBool>,
    active_requests: Arc<AtomicU64>,
}

struct ActiveRequestGuard(Arc<AtomicU64>);

impl ActiveRequestGuard {
    fn new(counter: Arc<AtomicU64>) -> Self {
        counter.fetch_add(1, Ordering::AcqRel);
        Self(counter)
    }
}

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[tonic::async_trait]
impl PluginRuntime for SenseVoiceWorker {
    async fn describe(
        &self,
        _request: Request<DescribeRequest>,
    ) -> Result<Response<DescribeResponse>, Status> {
        Ok(Response::new(DescribeResponse {
            plugin_id: "sensevoice-asr".into(),
            plugin_version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: PROTOCOL_VERSION,
            capabilities: vec!["audio.transcribe".into()],
            npu_required: true,
        }))
    }

    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        let loaded = self.engine.lock().map_err(internal)?.is_some();
        let serving = loaded && !self.draining.load(Ordering::Acquire);
        Ok(Response::new(HealthResponse {
            status: if serving {
                ServingStatus::Serving
            } else {
                ServingStatus::NotServing
            } as i32,
            detail: if serving { "ready" } else { "not ready" }.into(),
            active_requests: self.active_requests.load(Ordering::Acquire),
        }))
    }

    async fn load(
        &self,
        request: Request<LoadRequest>,
    ) -> Result<Response<LoadResponse>, Status> {
        let request = request.into_inner();
        if request.lease_id.is_empty() || request.core_mask == 0 {
            return Err(Status::invalid_argument(
                "lease_id and core_mask are required",
            ));
        }
        let core_mask = request.core_mask;
        let engine = tokio::task::spawn_blocking(move || Engine::load(core_mask))
            .await
            .map_err(internal)?
            .map_err(Status::failed_precondition)?;
        let runtime_version = engine.runtime_version.clone();
        let driver_version = engine.driver_version.clone();
        *self.engine.lock().map_err(internal)? = Some(engine);
        self.draining.store(false, Ordering::Release);
        Ok(Response::new(LoadResponse {
            runtime_version,
            driver_version,
            model_target: "rk3576".into(),
            active_core_mask: core_mask,
        }))
    }

    async fn execute_job(
        &self,
        request: Request<ExecuteJobRequest>,
    ) -> Result<Response<ExecuteJobResponse>, Status> {
        if self.draining.load(Ordering::Acquire) {
            return Err(Status::unavailable("worker is draining"));
        }
        let request = request.into_inner();
        if request.job_id.is_empty() || request.capability_id != "audio.transcribe" {
            return Err(Status::invalid_argument("invalid transcription job"));
        }
        if !matches!(
            request.content_type.as_str(),
            "audio/wav" | "audio/x-wav" | "audio/wave"
        ) {
            return Err(Status::invalid_argument("content type must be WAV audio"));
        }
        if request.payload.is_empty() {
            return Err(Status::invalid_argument("audio input is empty"));
        }
        let language = request
            .parameters
            .get("language")
            .map(String::as_str)
            .unwrap_or("auto");
        if !matches!(language, "auto" | "zh" | "en" | "yue" | "ja" | "ko") {
            return Err(Status::invalid_argument("unsupported SenseVoice language"));
        }
        let normalization = request
            .parameters
            .get("text_normalization")
            .map(String::as_str)
            .unwrap_or("withitn");
        if !matches!(normalization, "withitn" | "woitn") {
            return Err(Status::invalid_argument(
                "text_normalization must be withitn or woitn",
            ));
        }

        let active = ActiveRequestGuard::new(self.active_requests.clone());
        let engine = self.engine.clone();
        let payload = request.payload;
        let language = language.to_owned();
        let with_itn = normalization == "withitn";
        let result = tokio::task::spawn_blocking(move || {
            let _active = active;
            let mut guard = engine.lock().map_err(|error| error.to_string())?;
            guard
                .as_mut()
                .ok_or_else(|| "model is not loaded".to_owned())?
                .transcribe(&payload, &language, with_itn)
        })
        .await
        .map_err(internal)?
        .map_err(Status::internal)?;

        Ok(Response::new(ExecuteJobResponse {
            content_type: "application/json".into(),
            payload: result.json,
            queue_us: 0,
            preprocess_us: result.timing.preprocess_us,
            inference_us: result.timing.inference_us,
            postprocess_us: result.timing.postprocess_us,
        }))
    }

    async fn drain(
        &self,
        _request: Request<DrainRequest>,
    ) -> Result<Response<DrainResponse>, Status> {
        self.draining.store(true, Ordering::Release);
        Ok(Response::new(DrainResponse {
            remaining_requests: self.active_requests.load(Ordering::Acquire),
        }))
    }

    async fn unload(
        &self,
        _request: Request<UnloadRequest>,
    ) -> Result<Response<UnloadResponse>, Status> {
        if self.active_requests.load(Ordering::Acquire) != 0 {
            return Err(Status::failed_precondition(
                "transcription is still active",
            ));
        }
        self.engine.lock().map_err(internal)?.take();
        Ok(Response::new(UnloadResponse {}))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();
    let socket = std::env::var_os("RKSERVE_WORKER_SOCKET")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("RKSERVE_WORKER_SOCKET is required"))?;
    if let Some(parent) = socket.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if socket.exists() {
        tokio::fs::remove_file(&socket).await?;
    }
    let listener = UnixListener::bind(&socket)?;
    info!(socket = %socket.display(), "SenseVoice ASR worker is ready for handshake");
    Server::builder()
        .add_service(
            PluginRuntimeServer::new(SenseVoiceWorker::default())
                .max_decoding_message_size(MAX_RPC_MESSAGE_SIZE)
                .max_encoding_message_size(MAX_RPC_MESSAGE_SIZE),
        )
        .serve_with_incoming_shutdown(UnixListenerStream::new(listener), shutdown_signal())
        .await?;
    let _ = tokio::fs::remove_file(&socket).await;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

fn plugin_asset(relative: &str) -> Result<PathBuf, String> {
    let plugin = std::env::var_os("RKSERVE_PLUGIN_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "RKSERVE_PLUGIN_DIR is required".to_owned())?;
    let path = plugin.join(relative);
    if !path.is_file() {
        return Err(format!("plugin asset is missing: {}", path.display()));
    }
    Ok(path)
}

fn c_path(path: PathBuf) -> Result<CString, String> {
    CString::new(path.as_os_str().to_string_lossy().as_bytes())
        .map_err(|_| "plugin path contains a NUL byte".to_owned())
}

fn read_c_string(buffer: &[c_char]) -> String {
    unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

fn internal(error: impl std::fmt::Display) -> Status {
    Status::internal(error.to_string())
}
