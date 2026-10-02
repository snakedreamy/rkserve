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
    token_count: u64,
}

unsafe extern "C" {
    fn rkserve_zipformer_asr_create(
        encoder_path: *const c_char,
        decoder_path: *const c_char,
        joiner_path: *const c_char,
        vocab_path: *const c_char,
        core_mask: i32,
        error: *mut c_char,
        error_capacity: usize,
    ) -> *mut NativeEngine;
    fn rkserve_zipformer_asr_destroy(engine: *mut NativeEngine);
    fn rkserve_zipformer_asr_transcribe(
        engine: *mut NativeEngine,
        wav: *const u8,
        wav_size: usize,
        beam_size: i32,
        text: *mut *mut c_char,
        text_size: *mut usize,
        timing: *mut NativeTiming,
        error: *mut c_char,
        error_capacity: usize,
    ) -> i32;
    fn rkserve_zipformer_asr_free(pointer: *mut c_void);
    fn rkserve_zipformer_asr_versions(
        engine: *mut NativeEngine,
        runtime: *mut c_char,
        runtime_capacity: usize,
        driver: *mut c_char,
        driver_capacity: usize,
    ) -> i32;
}

struct Transcription {
    text: Vec<u8>,
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
        let encoder = c_path(plugin_asset("assets/models/encoder.rknn")?)?;
        let decoder = c_path(plugin_asset("assets/models/decoder.rknn")?)?;
        let joiner = c_path(plugin_asset("assets/models/joiner.rknn")?)?;
        let vocab = c_path(plugin_asset("assets/text/vocab.txt")?)?;
        let mut error = [0 as c_char; 1024];
        let native = unsafe {
            rkserve_zipformer_asr_create(
                encoder.as_ptr(),
                decoder.as_ptr(),
                joiner.as_ptr(),
                vocab.as_ptr(),
                core_mask,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        let native = NonNull::new(native).ok_or_else(|| read_c_string(&error))?;

        let mut runtime = [0 as c_char; 256];
        let mut driver = [0 as c_char; 128];
        if unsafe {
            rkserve_zipformer_asr_versions(
                native.as_ptr(),
                runtime.as_mut_ptr(),
                runtime.len(),
                driver.as_mut_ptr(),
                driver.len(),
            )
        } != 0
        {
            unsafe { rkserve_zipformer_asr_destroy(native.as_ptr()) };
            return Err("could not query RKNN runtime versions".into());
        }
        Ok(Self {
            native,
            runtime_version: read_c_string(&runtime),
            driver_version: read_c_string(&driver),
        })
    }

    fn transcribe(&mut self, wav: &[u8], beam_size: i32) -> Result<Transcription, String> {
        let mut text = std::ptr::null_mut();
        let mut text_size = 0usize;
        let mut timing = NativeTiming::default();
        let mut error = [0 as c_char; 1024];
        let result = unsafe {
            rkserve_zipformer_asr_transcribe(
                self.native.as_ptr(),
                wav.as_ptr(),
                wav.len(),
                beam_size,
                &mut text,
                &mut text_size,
                &mut timing,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if result != 0 {
            if !text.is_null() {
                unsafe { rkserve_zipformer_asr_free(text.cast()) };
            }
            return Err(read_c_string(&error));
        }
        if text.is_null() && text_size != 0 {
            return Err("native engine returned an invalid text buffer".into());
        }
        let bytes = if text_size == 0 {
            Vec::new()
        } else {
            unsafe { slice::from_raw_parts(text.cast::<u8>(), text_size) }.to_vec()
        };
        if !text.is_null() {
            unsafe { rkserve_zipformer_asr_free(text.cast()) };
        }
        std::str::from_utf8(&bytes)
            .map_err(|_| "native engine returned invalid UTF-8".to_owned())?;
        Ok(Transcription { text: bytes, timing })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { rkserve_zipformer_asr_destroy(self.native.as_ptr()) };
    }
}

#[derive(Clone, Default)]
struct ZipformerWorker {
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
impl PluginRuntime for ZipformerWorker {
    async fn describe(
        &self,
        _request: Request<DescribeRequest>,
    ) -> Result<Response<DescribeResponse>, Status> {
        Ok(Response::new(DescribeResponse {
            plugin_id: "zipformer-asr".into(),
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
            return Err(Status::invalid_argument("lease_id and core_mask are required"));
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
        let beam_size = request
            .parameters
            .get("beam_size")
            .map(|value| {
                value.parse::<i32>().map_err(|_| {
                    Status::invalid_argument("beam_size must be an integer between 1 and 8")
                })
            })
            .transpose()?
            .unwrap_or(4);
        if !(1..=8).contains(&beam_size) {
            return Err(Status::invalid_argument(
                "beam_size must be an integer between 1 and 8",
            ));
        }

        let active = ActiveRequestGuard::new(self.active_requests.clone());
        let engine = self.engine.clone();
        let payload = request.payload;
        let result = tokio::task::spawn_blocking(move || {
            let _active = active;
            let mut guard = engine.lock().map_err(|error| error.to_string())?;
            guard
                .as_mut()
                .ok_or_else(|| "model is not loaded".to_owned())?
                .transcribe(&payload, beam_size)
        })
        .await
        .map_err(internal)?
        .map_err(Status::internal)?;

        Ok(Response::new(ExecuteJobResponse {
            content_type: "text/plain; charset=utf-8".into(),
            payload: result.text,
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
            return Err(Status::failed_precondition("transcription is still active"));
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
    info!(socket = %socket.display(), "Zipformer ASR worker is ready for handshake");
    Server::builder()
        .add_service(
            PluginRuntimeServer::new(ZipformerWorker::default())
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
