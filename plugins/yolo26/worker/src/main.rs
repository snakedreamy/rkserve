use std::{
    collections::BTreeMap,
    ffi::{CStr, CString, c_char},
    fs,
    path::{Component, Path, PathBuf},
    ptr::NonNull,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

use rkserve_protocol::{
    PROTOCOL_VERSION,
    plugin::v1::{
        DescribeRequest, DescribeResponse, DrainRequest, DrainResponse, ExecuteJobRequest,
        ExecuteJobResponse, HealthRequest, HealthResponse, LoadRequest, LoadResponse,
        UnloadRequest, UnloadResponse,
        health_response::ServingStatus,
        plugin_runtime_server::{PluginRuntime, PluginRuntimeServer},
    },
};
use serde::{Deserialize, Serialize};
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::{Request, Response, Status, transport::Server};
use tracing::{info, warn};

const MAX_DETECTIONS: usize = 300;
const MAX_RPC_MESSAGE_SIZE: usize = 64 * 1024 * 1024;
const DEFAULT_CONFIDENCE: f32 = 0.25;
const DEFAULT_IOU: f32 = 0.70;
const DEFAULT_MAX_DETECTIONS: usize = 300;

#[repr(C)]
struct NativeDetection {
    class_id: i32,
    confidence: f32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

#[derive(Default)]
#[repr(C)]
struct NativeTiming {
    preprocess_us: u64,
    inference_us: u64,
    postprocess_us: u64,
    rga_used: u8,
}

enum NativeEngine {}

unsafe extern "C" {
    fn rkserve_yolo26_create(
        model_path: *const c_char,
        class_count: i32,
        core_mask: i32,
        error: *mut c_char,
        error_capacity: usize,
    ) -> *mut NativeEngine;
    fn rkserve_yolo26_destroy(engine: *mut NativeEngine);
    fn rkserve_yolo26_infer(
        engine: *mut NativeEngine,
        rgb: *const u8,
        width: i32,
        height: i32,
        confidence_threshold: f32,
        iou_threshold: f32,
        max_detections: i32,
        detections: *mut NativeDetection,
        capacity: i32,
        timing: *mut NativeTiming,
        error: *mut c_char,
        error_capacity: usize,
    ) -> i32;
    fn rkserve_yolo26_versions(
        engine: *mut NativeEngine,
        api_version: *mut c_char,
        api_capacity: usize,
        driver_version: *mut c_char,
        driver_capacity: usize,
    ) -> i32;
}

struct Engine {
    native: NonNull<NativeEngine>,
    runtime_version: String,
    driver_version: String,
    model_name: String,
    labels: Vec<String>,
}

unsafe impl Send for Engine {}

struct InferenceResult {
    detections: Vec<Detection>,
    preprocess_us: u64,
    inference_us: u64,
    postprocess_us: u64,
}

impl Engine {
    fn load(model: ModelAsset, core_mask: i32) -> Result<Self, String> {
        let model_path = model.absolute_path;
        let path = CString::new(model_path).map_err(|_| "model path contains a NUL byte")?;
        let mut error = [0 as c_char; 512];
        let native = unsafe {
            rkserve_yolo26_create(
                path.as_ptr(),
                model.labels.len() as i32,
                core_mask,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        let native = NonNull::new(native).ok_or_else(|| read_c_string(&error))?;

        let mut runtime = [0 as c_char; 128];
        let mut driver = [0 as c_char; 128];
        let result = unsafe {
            rkserve_yolo26_versions(
                native.as_ptr(),
                runtime.as_mut_ptr(),
                runtime.len(),
                driver.as_mut_ptr(),
                driver.len(),
            )
        };
        if result != 0 {
            unsafe { rkserve_yolo26_destroy(native.as_ptr()) };
            return Err("could not query RKNN runtime versions".into());
        }
        Ok(Self {
            native,
            runtime_version: read_c_string(&runtime),
            driver_version: read_c_string(&driver),
            model_name: model.relative_path,
            labels: model.labels,
        })
    }

    fn infer(
        &mut self,
        payload: &[u8],
        confidence: f32,
        iou: f32,
        max_detections: usize,
    ) -> Result<InferenceResult, String> {
        let decode_started = Instant::now();
        let image = image::load_from_memory(payload)
            .map_err(|error| format!("decode image: {error}"))?
            .into_rgb8();
        let decode_us = decode_started.elapsed().as_micros() as u64;
        let width = i32::try_from(image.width()).map_err(|_| "image width exceeds i32")?;
        let height = i32::try_from(image.height()).map_err(|_| "image height exceeds i32")?;
        let mut native: Vec<NativeDetection> = (0..max_detections)
            .map(|_| NativeDetection {
                class_id: 0,
                confidence: 0.0,
                left: 0.0,
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
            })
            .collect();
        let mut timing = NativeTiming::default();
        let mut error = [0 as c_char; 512];
        let count = unsafe {
            rkserve_yolo26_infer(
                self.native.as_ptr(),
                image.as_ptr(),
                width,
                height,
                confidence,
                iou,
                max_detections as i32,
                native.as_mut_ptr(),
                native.len() as i32,
                &mut timing,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if count < 0 {
            return Err(read_c_string(&error));
        }
        if timing.rga_used == 0 {
            warn!("RGA letterbox failed; this request used the CPU fallback");
        }
        Ok(InferenceResult {
            detections: native
                .into_iter()
                .take(count as usize)
                .map(|item| Detection {
                    class_id: item.class_id,
                    label: self
                        .labels
                        .get(item.class_id as usize)
                        .cloned()
                        .unwrap_or_else(|| "unknown".to_owned()),
                    confidence: item.confidence,
                    box_: BoundingBox {
                        left: item.left,
                        top: item.top,
                        right: item.right,
                        bottom: item.bottom,
                    },
                })
                .collect(),
            preprocess_us: decode_us + timing.preprocess_us,
            inference_us: timing.inference_us,
            postprocess_us: timing.postprocess_us,
        })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { rkserve_yolo26_destroy(self.native.as_ptr()) };
    }
}

#[derive(Serialize)]
struct DetectionResult {
    model: String,
    detections: Vec<Detection>,
}

#[derive(Serialize)]
struct Detection {
    class_id: i32,
    label: String,
    confidence: f32,
    #[serde(rename = "box")]
    box_: BoundingBox,
}

#[derive(Serialize)]
struct BoundingBox {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

#[derive(Clone)]
struct Yolo26Worker {
    engine: Arc<Mutex<Option<Engine>>>,
    draining: Arc<AtomicBool>,
    active_requests: Arc<AtomicU64>,
}

impl Default for Yolo26Worker {
    fn default() -> Self {
        Self {
            engine: Arc::new(Mutex::new(None)),
            draining: Arc::new(AtomicBool::new(false)),
            active_requests: Arc::new(AtomicU64::new(0)),
        }
    }
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
impl PluginRuntime for Yolo26Worker {
    async fn describe(
        &self,
        _request: Request<DescribeRequest>,
    ) -> Result<Response<DescribeResponse>, Status> {
        Ok(Response::new(DescribeResponse {
            plugin_id: "yolo26".into(),
            plugin_version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: PROTOCOL_VERSION,
            capabilities: vec!["vision.detect".into()],
            npu_required: true,
        }))
    }

    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        let guard = self.engine.lock().map_err(internal)?;
        let loaded = guard.is_some();
        let model = guard.as_ref().map(|engine| engine.model_name.as_str());
        let serving = loaded && !self.draining.load(Ordering::Acquire);
        Ok(Response::new(HealthResponse {
            status: if serving {
                ServingStatus::Serving
            } else {
                ServingStatus::NotServing
            } as i32,
            detail: if serving {
                format!("ready ({})", model.unwrap_or("unknown"))
            } else {
                "not ready".into()
            },
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
        let selected_model = request
            .config
            .get("model")
            .ok_or_else(|| Status::invalid_argument("model configuration is required"))?;
        let model = plugin_model(selected_model)?;
        let core_mask = request.core_mask;
        let engine = tokio::task::spawn_blocking(move || Engine::load(model, core_mask))
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
            active_core_mask: request.core_mask,
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
        if request.job_id.is_empty() {
            return Err(Status::invalid_argument("job_id is required"));
        }
        if request.capability_id != "vision.detect" {
            return Err(Status::invalid_argument("unsupported capability"));
        }
        let confidence = optional_number(
            request.parameters.get("confidence"),
            "confidence",
            0.01,
            1.0,
        )?
        .unwrap_or(DEFAULT_CONFIDENCE);
        let iou = optional_number(request.parameters.get("iou"), "iou", 0.0, 1.0)?
            .unwrap_or(DEFAULT_IOU);
        let max_detections = optional_integer(
            request.parameters.get("max_detections"),
            "max_detections",
            1,
            MAX_DETECTIONS,
        )?
        .unwrap_or(DEFAULT_MAX_DETECTIONS);

        let active_request = ActiveRequestGuard::new(self.active_requests.clone());
        let engine = self.engine.clone();
        let payload = request.payload;
        let (result, model_name) = tokio::task::spawn_blocking(move || {
            let _active_request = active_request;
            let mut guard = engine.lock().map_err(|error| error.to_string())?;
            let engine = guard
                .as_mut()
                .ok_or_else(|| "model is not loaded".to_owned())?;
            let model_name = engine.model_name.clone();
            let result = engine.infer(&payload, confidence, iou, max_detections)?;
            Ok::<_, String>((result, model_name))
        })
        .await
        .map_err(internal)?
        .map_err(Status::internal)?;
        let response_payload = serde_json::to_vec(&DetectionResult {
            model: model_name,
            detections: result.detections,
        })
        .map_err(internal)?;
        Ok(Response::new(ExecuteJobResponse {
            content_type: "application/json".into(),
            payload: response_payload,
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
            return Err(Status::failed_precondition("inference is still active"));
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
    info!(socket = %socket.display(), "YOLO26 worker is ready for handshake");
    Server::builder()
        .add_service(
            PluginRuntimeServer::new(Yolo26Worker::default())
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

struct ModelAsset {
    relative_path: String,
    absolute_path: String,
    labels: Vec<String>,
}

#[derive(Deserialize)]
struct ModelMetadata {
    task: String,
    imgsz: ImageSize,
    names: ClassNames,
    #[serde(default)]
    end2end: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ImageSize {
    Square(u32),
    Dimensions(Vec<u32>),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ClassNames {
    Indexed(BTreeMap<usize, String>),
    Ordered(Vec<String>),
}

fn plugin_model(relative_path: &str) -> Result<ModelAsset, Status> {
    let relative = Path::new(relative_path);
    if relative.is_absolute()
        || relative.extension().and_then(|value| value.to_str()) != Some("rknn")
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(Status::invalid_argument("model must be a discovered RKNN asset"));
    }
    let plugin_dir = std::env::var_os("RKSERVE_PLUGIN_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| Status::failed_precondition("RKSERVE_PLUGIN_DIR is required"))?;
    let models_root = plugin_dir.join("assets/models").canonicalize().map_err(|error| {
        Status::failed_precondition(format!("canonicalize plugin model directory: {error}"))
    })?;
    let model_path = models_root.join(relative).canonicalize().map_err(|error| {
        Status::failed_precondition(format!("canonicalize selected model: {error}"))
    })?;
    if !model_path.starts_with(&models_root) || !model_path.is_file() {
        return Err(Status::invalid_argument("selected model escapes the plugin model directory"));
    }

    let metadata_path = model_path
        .parent()
        .ok_or_else(|| Status::failed_precondition("selected model has no parent directory"))?
        .join("metadata.yaml");
    let metadata_raw = fs::read_to_string(&metadata_path).map_err(|error| {
        Status::failed_precondition(format!("read {}: {error}", metadata_path.display()))
    })?;
    let metadata: ModelMetadata = serde_yaml::from_str(&metadata_raw).map_err(|error| {
        Status::failed_precondition(format!("parse {}: {error}", metadata_path.display()))
    })?;
    if metadata.task != "detect" || metadata.end2end {
        return Err(Status::failed_precondition(
            "model metadata must describe a non-end-to-end detect model",
        ));
    }
    let dimensions = match metadata.imgsz {
        ImageSize::Square(size) => vec![size, size],
        ImageSize::Dimensions(dimensions) => dimensions,
    };
    if dimensions.as_slice() != [640, 640] {
        return Err(Status::failed_precondition(
            "YOLO26 plugin currently requires 640x640 model metadata",
        ));
    }
    let labels = metadata.names.into_labels()?;
    let absolute_path = model_path
        .into_os_string()
        .into_string()
        .map_err(|_| Status::failed_precondition("plugin model path is not valid UTF-8"))?;
    Ok(ModelAsset {
        relative_path: relative_path.to_owned(),
        absolute_path,
        labels,
    })
}

impl ClassNames {
    fn into_labels(self) -> Result<Vec<String>, Status> {
        let labels = match self {
            Self::Ordered(labels) => labels,
            Self::Indexed(labels) => {
                let mut ordered = Vec::with_capacity(labels.len());
                for index in 0..labels.len() {
                    ordered.push(labels.get(&index).cloned().ok_or_else(|| {
                        Status::failed_precondition(format!(
                            "model metadata is missing class name {index}"
                        ))
                    })?);
                }
                ordered
            }
        };
        if labels.is_empty()
            || labels.len() > 4096
            || labels.iter().any(|label| label.trim().is_empty())
        {
            return Err(Status::failed_precondition(
                "model metadata class names are empty or invalid",
            ));
        }
        Ok(labels)
    }
}

fn optional_number(
    value: Option<&String>,
    name: &str,
    minimum: f32,
    maximum: f32,
) -> Result<Option<f32>, Status> {
    let Some(value) = value else {
        return Ok(None);
    };
    let number = value.parse::<f32>().map_err(|_| {
        Status::invalid_argument(format!("parameter '{name}' must be a number"))
    })?;
    if !number.is_finite() || number < minimum || number > maximum {
        return Err(Status::invalid_argument(format!(
            "parameter '{name}' must be between {minimum} and {maximum}"
        )));
    }
    Ok(Some(number))
}

fn optional_integer(
    value: Option<&String>,
    name: &str,
    minimum: usize,
    maximum: usize,
) -> Result<Option<usize>, Status> {
    let Some(value) = value else {
        return Ok(None);
    };
    let number = value.parse::<usize>().map_err(|_| {
        Status::invalid_argument(format!("parameter '{name}' must be an integer"))
    })?;
    if number < minimum || number > maximum {
        return Err(Status::invalid_argument(format!(
            "parameter '{name}' must be between {minimum} and {maximum}"
        )));
    }
    Ok(Some(number))
}

fn read_c_string(buffer: &[c_char]) -> String {
    unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

fn internal(error: impl std::fmt::Display) -> Status {
    Status::internal(error.to_string())
}
