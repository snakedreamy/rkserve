// Naming convention:
// - Fields that mirror backend JSON payloads (via fetch/JSON.parse) use snake_case
//   to stay identical to the server contract and avoid error-prone manual mapping.
// - Fields introduced or composed on the frontend (e.g. parsed from HTTP headers,
//   or built from multiple values) use camelCase per TypeScript convention.

export type CoreMask = 'core0' | 'core1' | 'core0_1' | 'auto'

export type KeyRole = 'management' | 'api'

export interface ApiPrincipal {
  name: string
  role?: KeyRole
  scopes: string[]
  fingerprint: string
}

export interface NpuCore {
  id: number
  label: string
  allocation_id: string | null
  plugin_id: string | null
}

export interface NpuTopology {
  device: string
  platform: string
  driver: string
  driver_version: string | null
  runtime_version: string | null
  total_tops_int8: number
  core_count: number
  current_frequency_hz: number | null
  available_frequencies_hz: number[]
  load_percent: number | null
  governor: string | null
  cores: NpuCore[]
}

export interface DeviceTelemetry {
  soc_temperature_c: number | null
  npu_temperature_c: number | null
  memory_total_bytes: number | null
  memory_available_bytes: number | null
  npu_core_load_percent: Array<number | null>
}

export interface Allocation {
  lease_id: string
  plugin_id: string
  core_mask: CoreMask
  created_at_unix_ms: number
}

export type PluginState = 'installed' | 'ready' | 'backoff' | 'failed'

export interface PluginSummary {
  translations?: Record<string, Record<string, string>>
  id: string
  name: string
  version: string
  state: PluginState
  allowed_masks: CoreMask[]
  default_mask: CoreMask
  max_concurrency: number
  queue_size: number
  request_timeout_ms: number
  capabilities: CapabilityDefinition[]
  configuration: ConfigurationField[]
  configuration_values: Record<string, string>
}

export type CapabilityInputKind = 'image' | 'audio' | 'text' | 'binary'
export type CapabilityOutputKind = 'detections' | 'audio' | 'text' | 'json' | 'binary'
export type CapabilityRenderer =
  | 'audio_player'
  | 'image_detection_overlay'
  | 'image_pose_overlay'
  | 'speech_transcription'

export interface CapabilityPresentation {
  renderer: CapabilityRenderer | null
  input_hint: string | null
  input_placeholder: string | null
  text_examples: string[]
}

export interface CapabilityDefinition {
  id: string
  name: string
  description: string
  input_kind: CapabilityInputKind
  accepted_content_types: string[]
  max_input_bytes: number
  output_kind: CapabilityOutputKind
  output_content_type: string
  presentation?: CapabilityPresentation
  parameters: ConfigurationField[]
}

export interface ConfigurationField {
  key: string
  label: string
  description: string
  kind: 'string' | 'number' | 'boolean' | 'select'
  required: boolean
  default: string | null
  options: string[]
  // Display-only labels; option values remain unchanged on the wire.
  optionLabels?: Record<string, string>
  options_from?: 'rknn_models'
  min: number | null
  max: number | null
}

export interface JobResult {
  contentType: string
  data: unknown
  blob: Blob
  rawText: string | null
  // Parsed response headers are frontend-owned values, hence camelCase.
  timings: {
    queueUs: number
    preprocessUs: number
    inferenceUs: number
    postprocessUs: number
  }
}

export interface PoseResult {
  model: string
  poses: Array<{
    confidence: number
    box: { left: number; top: number; right: number; bottom: number }
    keypoints: Array<{
      id: number
      label: string
      x: number
      y: number
      confidence: number
    }>
  }>
}

export type JobState = 'queued' | 'running' | 'canceling' | 'succeeded' | 'failed' | 'canceled'

export interface InferenceJob {
  id: string
  plugin_id: string
  capability_id: string
  state: JobState
  created_at_unix_ms: number
  started_at_unix_ms: number | null
  finished_at_unix_ms: number | null
  cancel_requested: boolean
  content_type: string | null
  result_bytes: number | null
  // Job snapshots mirror the backend JSON contract and retain snake_case.
  timings: {
    queue_us: number
    preprocess_us: number
    inference_us: number
    postprocess_us: number
  } | null
  error: string | null
  request_id: string | null
  submitted_by: string | null
  input_sha256: string | null
  result_sha256: string | null
}

export type WorkerStatus = 'ready' | 'backoff'

export interface WorkerSnapshot {
  lease_id: string
  plugin_id: string
  pid: number | null
  runtime_version: string
  driver_version: string
  model_target: string
  core_mask: CoreMask
  status: WorkerStatus
  restart_attempt: number
  last_error: string | null
}

export type EventSeverity = 'info' | 'warning' | 'error'

export interface RuntimeEvent {
  id: number
  timestamp_unix_ms: number
  severity: EventSeverity
  category: string
  kind: string
  message: string
  metadata: Record<string, unknown> | null
  plugin_id: string | null
  lease_id: string | null
  job_id: string | null
  request_id: string | null
  source: string | null
  actor: string | null
  client_ip: string | null
  peer_ip: string | null
  http_method: string | null
  http_path: string | null
  http_status: number | null
  duration_us: number | null
  request_bytes: number | null
  response_bytes: number | null
}

export interface EventPage {
  items: RuntimeEvent[]
  next_cursor: number | null
}

export interface EventFilters {
  after?: number
  cursor?: number
  severity?: EventSeverity
  category?: string
  kind?: string
  pluginId?: string
  leaseId?: string
  jobId?: string
  requestId?: string
  actor?: string
  clientIp?: string
  httpMethod?: string
  httpStatus?: number
  source?: string
  excludeSource?: string
  q?: string
  outcome?: 'success' | 'failure'
  pathPrefix?: string
  from?: number
  to?: number
  limit?: number
}

export interface ApiOperation {
  method: string
  path: string
  description: string
}

export interface PluginSpec {
  plugin: PluginSummary
  jobs: {
    submit: ApiOperation
    get: ApiOperation
    cancel: ApiOperation
    result: ApiOperation
    capabilities: CapabilityDefinition[]
  }
}

export interface DetectionResult {
  model: string
  detections: Array<{
    class_id: number
    label: string
    confidence: number
    box: { left: number; top: number; right: number; bottom: number }
  }>
}
