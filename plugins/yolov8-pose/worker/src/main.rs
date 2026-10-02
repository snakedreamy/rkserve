use std::{
    ffi::{CStr, CString, c_char},
    path::PathBuf,
    ptr::NonNull,
    sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, Ordering}},
    time::Instant,
};

use rkserve_protocol::{PROTOCOL_VERSION, plugin::v1::{
    DescribeRequest, DescribeResponse, DrainRequest, DrainResponse, HealthRequest, HealthResponse,
    ExecuteJobRequest, ExecuteJobResponse, LoadRequest, LoadResponse, UnloadRequest, UnloadResponse,
    health_response::ServingStatus,
    plugin_runtime_server::{PluginRuntime, PluginRuntimeServer},
}};
use serde::Serialize;
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::{Request, Response, Status, transport::Server};
use tracing::info;

const MAX_POSES: usize = 128;
const MAX_RPC_MESSAGE_SIZE: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct NativeKeypoint {
    x: f32,
    y: f32,
    confidence: f32,
}

#[repr(C)]
struct NativePose {
    confidence: f32,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    keypoints: [NativeKeypoint; 17],
}

enum NativeEngine {}

unsafe extern "C" {
    fn rkserve_yolov8_pose_create(
        model_path: *const c_char,
        core_mask: i32,
        error: *mut c_char,
        error_capacity: usize,
    ) -> *mut NativeEngine;
    fn rkserve_yolov8_pose_destroy(engine: *mut NativeEngine);
    fn rkserve_yolov8_pose_infer(
        engine: *mut NativeEngine,
        rgb: *const u8,
        width: i32,
        height: i32,
        poses: *mut NativePose,
        capacity: i32,
        error: *mut c_char,
        error_capacity: usize,
    ) -> i32;
    fn rkserve_yolov8_pose_versions(
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
}

unsafe impl Send for Engine {}

impl Engine {
    fn load(model_path: &str, core_mask: i32) -> Result<Self, String> {
        let path = CString::new(model_path).map_err(|_| "model path contains a NUL byte")?;
        let mut error = [0 as c_char; 512];
        let native = unsafe {
            rkserve_yolov8_pose_create(path.as_ptr(), core_mask, error.as_mut_ptr(), error.len())
        };
        let native = NonNull::new(native).ok_or_else(|| read_c_string(&error))?;

        let mut runtime = [0 as c_char; 128];
        let mut driver = [0 as c_char; 128];
        let result = unsafe {
            rkserve_yolov8_pose_versions(
                native.as_ptr(),
                runtime.as_mut_ptr(),
                runtime.len(),
                driver.as_mut_ptr(),
                driver.len(),
            )
        };
        if result != 0 {
            unsafe { rkserve_yolov8_pose_destroy(native.as_ptr()) };
            return Err("could not query RKNN runtime versions".into());
        }
        Ok(Self {
            native,
            runtime_version: read_c_string(&runtime),
            driver_version: read_c_string(&driver),
        })
    }

    fn infer(&mut self, payload: &[u8]) -> Result<Vec<Pose>, String> {
        let image = image::load_from_memory(payload)
            .map_err(|error| format!("decode image: {error}"))?
            .into_rgb8();
        let width = image.width() as i32;
        let height = image.height() as i32;
        let mut native: Vec<NativePose> = (0..MAX_POSES)
            .map(|_| NativePose {
                confidence: 0.0,
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
                keypoints: [NativeKeypoint::default(); 17],
            })
            .collect();
        let mut error = [0 as c_char; 512];
        let count = unsafe {
            rkserve_yolov8_pose_infer(
                self.native.as_ptr(),
                image.as_ptr(),
                width,
                height,
                native.as_mut_ptr(),
                native.len() as i32,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if count < 0 {
            return Err(read_c_string(&error));
        }
        Ok(native
            .into_iter()
            .take(count as usize)
            .map(|item| Pose {
                confidence: item.confidence,
                box_: BoundingBox {
                    left: item.left,
                    top: item.top,
                    right: item.right,
                    bottom: item.bottom,
                },
                keypoints: item
                    .keypoints
                    .into_iter()
                    .enumerate()
                    .map(|(index, point)| Keypoint {
                        id: index,
                        label: KEYPOINT_LABELS[index],
                        x: point.x,
                        y: point.y,
                        confidence: point.confidence,
                    })
                    .collect(),
            })
            .collect())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { rkserve_yolov8_pose_destroy(self.native.as_ptr()) };
    }
}

#[derive(Serialize)]
struct PoseResult {
    model: &'static str,
    poses: Vec<Pose>,
}

#[derive(Serialize)]
struct Pose {
    confidence: f32,
    #[serde(rename = "box")]
    box_: BoundingBox,
    keypoints: Vec<Keypoint>,
}

#[derive(Serialize)]
struct Keypoint {
    id: usize,
    label: &'static str,
    x: f32,
    y: f32,
    confidence: f32,
}

#[derive(Serialize)]
struct BoundingBox {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[derive(Clone)]
struct PoseWorker {
    engine: Arc<Mutex<Option<Engine>>>,
    draining: Arc<AtomicBool>,
    active_requests: Arc<AtomicU64>,
}

impl Default for PoseWorker {
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
impl PluginRuntime for PoseWorker {
    async fn describe(&self, _request: Request<DescribeRequest>) -> Result<Response<DescribeResponse>, Status> {
        Ok(Response::new(DescribeResponse {
            plugin_id: "yolov8-pose".into(),
            plugin_version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: PROTOCOL_VERSION,
            capabilities: vec!["vision.pose".into()],
            npu_required: true,
        }))
    }

    async fn health(&self, _request: Request<HealthRequest>) -> Result<Response<HealthResponse>, Status> {
        let loaded = self.engine.lock().map_err(internal)?.is_some();
        let serving = loaded && !self.draining.load(Ordering::Acquire);
        Ok(Response::new(HealthResponse {
            status: if serving { ServingStatus::Serving } else { ServingStatus::NotServing } as i32,
            detail: if serving { "ready" } else { "not ready" }.into(),
            active_requests: self.active_requests.load(Ordering::Acquire),
        }))
    }

    async fn load(&self, request: Request<LoadRequest>) -> Result<Response<LoadResponse>, Status> {
        let request = request.into_inner();
        if request.lease_id.is_empty() || request.core_mask == 0 {
            return Err(Status::invalid_argument("lease_id and core_mask are required"));
        }
        let model_path = plugin_model_path("yolov8_pose.rknn")?;
        let engine = tokio::task::spawn_blocking(move || Engine::load(&model_path, request.core_mask))
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

    async fn execute_job(&self, request: Request<ExecuteJobRequest>) -> Result<Response<ExecuteJobResponse>, Status> {
        if self.draining.load(Ordering::Acquire) {
            return Err(Status::unavailable("worker is draining"));
        }
        let request = request.into_inner();
        if request.job_id.is_empty() {
            return Err(Status::invalid_argument("job_id is required"));
        }
        if request.capability_id != "vision.pose" {
            return Err(Status::invalid_argument("unsupported capability"));
        }
        let started = Instant::now();
        let active_request = ActiveRequestGuard::new(self.active_requests.clone());
        let engine = self.engine.clone();
        let payload = request.payload;
        let poses = tokio::task::spawn_blocking(move || {
            let _active_request = active_request;
            let mut guard = engine.lock().map_err(|error| error.to_string())?;
            guard.as_mut().ok_or_else(|| "model is not loaded".to_owned())?.infer(&payload)
        })
        .await
        .map_err(internal)?
        .map_err(Status::internal)?;
        let payload = serde_json::to_vec(&PoseResult { model: "yolov8n-pose", poses })
            .map_err(internal)?;
        Ok(Response::new(ExecuteJobResponse {
            content_type: "application/json".into(),
            payload,
            queue_us: 0,
            preprocess_us: 0,
            inference_us: started.elapsed().as_micros() as u64,
            postprocess_us: 0,
        }))
    }

    async fn drain(&self, _request: Request<DrainRequest>) -> Result<Response<DrainResponse>, Status> {
        self.draining.store(true, Ordering::Release);
        Ok(Response::new(DrainResponse {
            remaining_requests: self.active_requests.load(Ordering::Acquire),
        }))
    }

    async fn unload(&self, _request: Request<UnloadRequest>) -> Result<Response<UnloadResponse>, Status> {
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
    info!(socket = %socket.display(), "YOLOv8 Pose worker is ready for handshake");
    Server::builder()
        .add_service(
            PluginRuntimeServer::new(PoseWorker::default())
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

fn plugin_model_path(file_name: &str) -> Result<String, Status> {
    let plugin_dir = std::env::var_os("RKSERVE_PLUGIN_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| Status::failed_precondition("RKSERVE_PLUGIN_DIR is required"))?;
    let path = plugin_dir.join("assets/models").join(file_name);
    if !path.is_file() {
        return Err(Status::failed_precondition(format!(
            "plugin model is missing: {}",
            path.display()
        )));
    }
    path.into_os_string()
        .into_string()
        .map_err(|_| Status::failed_precondition("plugin model path is not valid UTF-8"))
}

fn read_c_string(buffer: &[c_char]) -> String {
    unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy().into_owned()
}

fn internal(error: impl std::fmt::Display) -> Status {
    Status::internal(error.to_string())
}

const KEYPOINT_LABELS: &[&str; 17] = &[
    "nose",
    "left_eye",
    "right_eye",
    "left_ear",
    "right_ear",
    "left_shoulder",
    "right_shoulder",
    "left_elbow",
    "right_elbow",
    "left_wrist",
    "right_wrist",
    "left_hip",
    "right_hip",
    "left_knee",
    "right_knee",
    "left_ankle",
    "right_ankle",
];
