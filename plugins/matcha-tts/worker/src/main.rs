use std::{
    ffi::{CStr, CString, c_char},
    path::PathBuf,
    ptr::NonNull,
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

const WAV_HEADER_SIZE: usize = 44;
const SAMPLE_RATE: u32 = 16_000;
static OUTPUT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

enum NativeEngine {}

unsafe extern "C" {
    fn rkserve_matcha_tts_create(
        matcha_path: *const c_char,
        vocos_path: *const c_char,
        lexicon_path: *const c_char,
        tokens_path: *const c_char,
        espeak_path: *const c_char,
        espeak_data_parent: *const c_char,
        ort_library_path: *const c_char,
        duration_model_path: *const c_char,
        core_mask: i32,
        error: *mut c_char,
        error_capacity: usize,
    ) -> *mut NativeEngine;
    fn rkserve_matcha_tts_destroy(engine: *mut NativeEngine);
    fn rkserve_matcha_tts_synthesize(
        engine: *mut NativeEngine,
        text: *const c_char,
        output_path: *const c_char,
        speed: f32,
        noise_scale: f32,
        output_samples: *mut i32,
        preprocess_us: *mut u64,
        inference_us: *mut u64,
        postprocess_us: *mut u64,
        error: *mut c_char,
        error_capacity: usize,
    ) -> i32;
    fn rkserve_matcha_tts_versions(
        engine: *mut NativeEngine,
        runtime: *mut c_char,
        runtime_capacity: usize,
        driver: *mut c_char,
        driver_capacity: usize,
    ) -> i32;
}

struct Synthesis {
    wav: Vec<u8>,
    preprocess_us: u64,
    inference_us: u64,
    postprocess_us: u64,
}

struct Engine {
    native: NonNull<NativeEngine>,
    runtime_version: String,
    driver_version: String,
    default_speed: f32,
    default_noise_scale: f32,
}

unsafe impl Send for Engine {}

impl Engine {
    fn load(core_mask: i32, default_speed: f32, default_noise_scale: f32) -> Result<Self, String> {
        let matcha = c_path(plugin_asset("assets/models/matcha-s64-fp16.rknn")?)?;
        let vocos = c_path(plugin_asset("assets/models/vocos-16khz-600-fp16.rknn")?)?;
        let lexicon = c_path(plugin_asset("assets/text/lexicon.txt")?)?;
        let tokens = c_path(plugin_asset("assets/text/tokens.txt")?)?;
        let espeak = c_path(plugin_asset("bin/espeak-ng")?)?;
        let data_parent = c_path(plugin_asset_dir("assets/text")?)?;
        let ort_library = c_path(plugin_asset("lib/libonnxruntime.so.1")?)?;
        let duration = c_path(plugin_asset("assets/models/matcha-duration.onnx")?)?;
        let mut error = [0 as c_char; 1024];
        let native = unsafe {
            rkserve_matcha_tts_create(
                matcha.as_ptr(),
                vocos.as_ptr(),
                lexicon.as_ptr(),
                tokens.as_ptr(),
                espeak.as_ptr(),
                data_parent.as_ptr(),
                ort_library.as_ptr(),
                duration.as_ptr(),
                core_mask,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        let native = NonNull::new(native).ok_or_else(|| read_c_string(&error))?;
        let mut runtime = [0 as c_char; 256];
        let mut driver = [0 as c_char; 128];
        if unsafe {
            rkserve_matcha_tts_versions(
                native.as_ptr(),
                runtime.as_mut_ptr(),
                runtime.len(),
                driver.as_mut_ptr(),
                driver.len(),
            )
        } != 0
        {
            unsafe { rkserve_matcha_tts_destroy(native.as_ptr()) };
            return Err("could not query native runtime versions".into());
        }
        Ok(Self {
            native,
            runtime_version: read_c_string(&runtime),
            driver_version: read_c_string(&driver),
            default_speed,
            default_noise_scale,
        })
    }

    fn synthesize(
        &mut self,
        text: &str,
        speed: Option<f32>,
        noise_scale: Option<f32>,
    ) -> Result<Synthesis, String> {
        let text = CString::new(text).map_err(|_| "speech input contains a NUL byte")?;
        let sequence = OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rkserve-matcha-tts-{}-{sequence}.wav",
            std::process::id()
        ));
        let output = c_path(path.clone())?;
        let mut error = [0 as c_char; 1024];
        let mut samples = 0;
        let mut preprocess_us = 0;
        let mut inference_us = 0;
        let mut postprocess_us = 0;
        let result = unsafe {
            rkserve_matcha_tts_synthesize(
                self.native.as_ptr(),
                text.as_ptr(),
                output.as_ptr(),
                speed.unwrap_or(self.default_speed),
                noise_scale.unwrap_or(self.default_noise_scale),
                &mut samples,
                &mut preprocess_us,
                &mut inference_us,
                &mut postprocess_us,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if result != 0 {
            let _ = std::fs::remove_file(&path);
            return Err(read_c_string(&error));
        }
        let wav = std::fs::read(&path).map_err(|error| format!("read WAV output: {error}"));
        let _ = std::fs::remove_file(&path);
        let wav = wav?;
        validate_wav(&wav, samples)?;
        Ok(Synthesis {
            wav,
            preprocess_us,
            inference_us,
            postprocess_us,
        })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { rkserve_matcha_tts_destroy(self.native.as_ptr()) };
    }
}

#[derive(Clone, Default)]
struct MatchaWorker {
    engine: Arc<Mutex<Option<Engine>>>,
    loaded: Arc<AtomicBool>,
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
impl PluginRuntime for MatchaWorker {
    async fn describe(
        &self,
        _request: Request<DescribeRequest>,
    ) -> Result<Response<DescribeResponse>, Status> {
        Ok(Response::new(DescribeResponse {
            plugin_id: "matcha-tts".into(),
            plugin_version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: PROTOCOL_VERSION,
            capabilities: vec!["audio.speech".into()],
            npu_required: true,
        }))
    }

    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        let serving = self.loaded.load(Ordering::Acquire)
            && !self.draining.load(Ordering::Acquire);
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
        let speed = configured_number(&request.config, "speed", 1.0, 0.7, 1.4)?;
        let noise_scale = configured_number(&request.config, "noise_scale", 0.667, 0.3, 1.0)?;
        let core_mask = request.core_mask;
        let engine = tokio::task::spawn_blocking(move || Engine::load(core_mask, speed, noise_scale))
            .await
            .map_err(internal)?
            .map_err(Status::failed_precondition)?;
        let runtime_version = engine.runtime_version.clone();
        let driver_version = engine.driver_version.clone();
        *self.engine.lock().map_err(internal)? = Some(engine);
        self.loaded.store(true, Ordering::Release);
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
        if request.job_id.is_empty() || request.capability_id != "audio.speech" {
            return Err(Status::invalid_argument("invalid speech job"));
        }
        if request.content_type != "text/plain" {
            return Err(Status::invalid_argument("content type must be text/plain"));
        }
        let text = std::str::from_utf8(&request.payload)
            .map_err(|_| Status::invalid_argument("speech input must be UTF-8"))?
            .trim();
        if text.is_empty() {
            return Err(Status::invalid_argument("speech input is empty"));
        }
        let speed = optional_number(request.parameters.get("speed"), "speed", 0.7, 1.4)?;
        let noise_scale = optional_number(
            request.parameters.get("noise_scale"),
            "noise_scale",
            0.3,
            1.0,
        )?;
        let text = text.to_owned();
        let active = ActiveRequestGuard::new(self.active_requests.clone());
        let engine = self.engine.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _active = active;
            let mut guard = engine.lock().map_err(|error| error.to_string())?;
            guard
                .as_mut()
                .ok_or_else(|| "model is not loaded".to_owned())?
                .synthesize(&text, speed, noise_scale)
        })
        .await
        .map_err(internal)?
        .map_err(Status::internal)?;
        Ok(Response::new(ExecuteJobResponse {
            content_type: "audio/wav".into(),
            payload: result.wav,
            queue_us: 0,
            preprocess_us: result.preprocess_us,
            inference_us: result.inference_us,
            postprocess_us: result.postprocess_us,
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
            return Err(Status::failed_precondition("speech synthesis is still active"));
        }
        self.engine.lock().map_err(internal)?.take();
        self.loaded.store(false, Ordering::Release);
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
    info!(socket = %socket.display(), "Matcha TTS worker is ready for handshake");
    Server::builder()
        .add_service(
            PluginRuntimeServer::new(MatchaWorker::default())
                .max_decoding_message_size(64 * 1024 * 1024)
                .max_encoding_message_size(64 * 1024 * 1024),
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

fn plugin_asset_dir(relative: &str) -> Result<PathBuf, String> {
    let plugin = std::env::var_os("RKSERVE_PLUGIN_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "RKSERVE_PLUGIN_DIR is required".to_owned())?;
    let path = plugin.join(relative);
    if !path.is_dir() {
        return Err(format!("plugin asset directory is missing: {}", path.display()));
    }
    Ok(path)
}

fn c_path(path: PathBuf) -> Result<CString, String> {
    CString::new(path.as_os_str().to_string_lossy().as_bytes())
        .map_err(|_| "plugin path contains a NUL byte".to_owned())
}

fn validate_wav(wav: &[u8], samples: i32) -> Result<(), String> {
    if samples <= 0
        || wav.len() < WAV_HEADER_SIZE
        || &wav[0..4] != b"RIFF"
        || &wav[8..12] != b"WAVE"
        || u16::from_le_bytes([wav[20], wav[21]]) != 1
        || u16::from_le_bytes([wav[22], wav[23]]) != 1
        || u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]) != SAMPLE_RATE
        || u16::from_le_bytes([wav[34], wav[35]]) != 16
    {
        return Err("native engine returned an invalid WAV file".into());
    }
    Ok(())
}

fn configured_number(
    config: &std::collections::HashMap<String, String>,
    key: &str,
    default: f32,
    min: f32,
    max: f32,
) -> Result<f32, Status> {
    optional_number(config.get(key), key, min, max).map(|value| value.unwrap_or(default))
}

fn optional_number(
    value: Option<&String>,
    key: &str,
    min: f32,
    max: f32,
) -> Result<Option<f32>, Status> {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    let parsed = value
        .parse::<f32>()
        .map_err(|_| Status::invalid_argument(format!("{key} must be a number")))?;
    if !parsed.is_finite() || parsed < min || parsed > max {
        return Err(Status::invalid_argument(format!(
            "{key} must be between {min} and {max}"
        )));
    }
    Ok(Some(parsed))
}

fn read_c_string(buffer: &[c_char]) -> String {
    unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

fn internal(error: impl std::fmt::Display) -> Status {
    Status::internal(error.to_string())
}
