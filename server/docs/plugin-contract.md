# RKServe plugin contract

English | [简体中文](plugin-contract.zh-CN.md)

## Admission requirements

An AI plugin must:

- Run its primary model inference through the Rockchip NPU runtime.
- Declare allowed core masks, concurrency, queue size and timeout.
- Report runtime/driver versions, model platform and actual core binding at initialization.
- Provide independent health, load, drain and unload operations.
- Report queue, NPU inference, CPU preprocessing and postprocessing timings.
- Keep models, runtime libraries, native sources and licenses within the plugin directory.

CPU-only applications cannot obtain NPU leases.

## Self-contained layout

An installed plugin package has this layout:

```text
<plugin-root>/<plugin-id>/
  plugin.toml
  bin/worker
  lib/
  assets/
  licenses/
```

`bin/worker` is the release ELF built by the plugin's `build.sh`; binaries, libraries and models are never committed to Git. Workers use `$ORIGIN/../lib` RPATH for private libraries and `RKSERVE_PLUGIN_DIR` to locate models.

## Manifest v2

```toml
schema_version = 2

[plugin]
id = "yolo26"
name = "YOLO26 Detection"
version = "0.1.0"
protocol_version = 2
executable = "bin/worker"

[[capabilities]]
id = "vision.detect"
name = "Object detection"
description = "Detect objects and return classes, confidence scores and bounding boxes."
input_kind = "image"
accepted_content_types = ["image/jpeg", "image/png"]
max_input_bytes = 10485760
output_kind = "detections"
output_content_type = "application/json"

[resources.npu]
allowed_masks = ["core0", "core1", "core0_1"]
default_mask = "core0_1"
max_concurrency = 1
queue_size = 16
request_timeout_ms = 30000
```

Core accepts only `schema_version = 2` and `protocol_version = 2`. `executable` must resolve to a file inside the plugin directory.

Exclusive leases, mandatory NPU execution and restart policy are fixed Core rules, not repeated manifest settings. Manifests cannot define `public_path`, arbitrary HTTP routes, frontend scripts or external model paths.

### Display translations

Base display fields are English. An optional top-level map adds localized strings without altering the protocol or values:

```toml
[translations.zh-CN]
"plugin.name" = "YOLO26 目标检测"
"capabilities.vision.detect.name" = "目标检测"
"capabilities.vision.detect.description" = "识别图像中的目标并返回类别、置信度和边界框。"
```

Keys are **flat quoted strings**; dots within a capability ID are not nested TOML paths. The API forwards the map as `translations: { "zh-CN": { "key": "value" } }`, omitting it when empty. Old manifests without translations remain valid. The console defaults to English, supports `zh-CN`, and falls back to base fields for missing translations.

Supported key patterns:

- `plugin.name`
- `capabilities.<id>.name` / `.description`
- `capabilities.<id>.presentation.input_hint` / `.input_placeholder`
- `capabilities.<id>.parameters.<key>.label` / `.description`
- `configuration.<key>.label` / `.description`
- Parameter/configuration prefixes followed by `.options.<raw-value>` for option display labels.

Translations are plain display text, not HTML. Never translate IDs, option/default values, model paths, renderer names or `text_examples`: examples are actual inference input. Core does not negotiate `Accept-Language`; API errors and event diagnostics remain English.

### Dynamic RKNN model discovery

Use `options_from = "rknn_models"` rather than hard-coding model variants:

```toml
[[configuration.fields]]
key = "model"
label = "Detection model"
description = "Select an RKNN model discovered in the plugin directory."
kind = "select"
required = true
options_from = "rknn_models"
```

At startup, Core recursively scans `assets/models/` for regular `.rknn` files with a sibling `metadata.yaml`, ignores symlinks, and uses paths relative to the model root as option values. The smallest file is the default. Values must belong to the discovery result, not arbitrary paths. Restart Core after adding/removing models to refresh the registry.

## Capabilities

A plugin must declare at least one capability. Core validates HTTP admission against the manifest and verifies matching capabilities during worker `Describe`.

- `id`: stable identifier passed as `ExecuteJobRequest.capability_id`.
- `input_kind`: `image`, `audio`, `text` or `binary`, selecting the input control.
- `accepted_content_types`: HTTP `Content-Type` allowlist.
- `max_input_bytes`: payload limit, no higher than Core's global cap.
- `output_kind`: `detections`, `audio`, `text`, `json` or `binary`, selecting the result renderer.
- `output_content_type`: successful result MIME type.

`[[capabilities.parameters]]` declares request fields: string, number, boolean or select, with defaults, ranges and options. Core validates them, the console builds forms, and workers read resolved values from `ExecuteJobRequest.parameters`.

### Declarative run UI

Capabilities can provide plain presentation metadata for controlled host renderers:

```toml
[capabilities.presentation]
renderer = "image_detection_overlay"
input_hint = "Upload a JPEG or PNG image; detection boxes will overlay the original."
```

Text capabilities may add `input_placeholder` and up to six `text_examples`. They render as plain text, never HTML or Markdown.

Supported renderers:

- `audio_player`: requires `output_kind = "audio"` and `audio/*` MIME; playback, replay and download.
- `image_detection_overlay`: image input with JSON detections; overlays boxes and retains details/raw JSON.
- `image_pose_overlay`: image input with JSON output; COCO 17-keypoint boxes, points and skeletons.
- `speech_transcription`: audio input with JSON output; text, language, emotion, sound events and long-audio segments.

Presentation metadata is optional. Without it, the console selects a renderer from `output_kind`. Plugins cannot provide JavaScript, React modules, HTML, CSS, remote URLs, dynamic imports or arbitrary component names.

`image_detection_overlay` uses RKServe detections v1:

```json
{
  "model": "yolo26n",
  "detections": [
    {
      "class_id": 0,
      "label": "person",
      "confidence": 0.92,
      "box": { "left": 16, "top": 24, "right": 300, "bottom": 420 }
    }
  ]
}
```

Coordinates must be in original-image pixels. The console clips out-of-bounds coordinates and ignores invalid boxes, but plugins must still produce valid results.

`image_pose_overlay` uses RKServe pose v1. `keypoints` must contain 17 points in COCO order, also in original-image pixels. The example below abbreviates the array:

```json
{
  "model": "yolov8n-pose",
  "poses": [
    {
      "confidence": 0.94,
      "box": { "left": 32, "top": 18, "right": 310, "bottom": 470 },
      "keypoints": [
        { "id": 0, "label": "nose", "x": 172.4, "y": 64.1, "confidence": 0.98 }
      ]
    }
  ]
}
```

`speech_transcription` keeps merged text and overall tags at the top level, with start/end times and per-segment results in `segments`. Output language is model data, not the console locale:

```json
{
  "text": "开放时间早上九点至下午五点。",
  "language": "zh",
  "emotion": "NEUTRAL",
  "events": ["Speech"],
  "audio_duration_ms": 5500,
  "segments": [
    {
      "index": 0,
      "start_ms": 0,
      "end_ms": 5500,
      "text": "开放时间早上九点至下午五点。",
      "language": "zh",
      "emotion": "NEUTRAL",
      "events": ["Speech"]
    }
  ]
}
```

Capabilities receive only the asynchronous namespace:

```text
POST   /api/v1/plugins/{plugin_id}/jobs/{capability_id}
GET    /api/v1/plugins/{plugin_id}/jobs/{job_id}
DELETE /api/v1/plugins/{plugin_id}/jobs/{job_id}
GET    /api/v1/plugins/{plugin_id}/jobs/{job_id}/result
GET    /api/v1/plugins/{plugin_id}/spec
```

Core generates `/spec` from the manifest for the console and external clients. Plugins cannot override Core routes.

## Configuration

Configuration fields share the parameter schema and reach the worker through `LoadRequest.config`. Changes while stopped are saved for the next start. Changes while running drain/unload the worker and reload it on the same core mask; new jobs may briefly receive worker-unavailable errors.

The same business key may appear in both scopes: `configuration.fields` defines instance defaults at load time, while `capabilities.parameters` provides per-job overrides. Matcha-TTS `speed` is an example; if the request omits an override, the plugin must use its instance configuration.

Configuration exposes user-adjustable runtime settings, not external model/library paths. Model and library files remain plugin-owned assets.

## Startup

1. Core validates the manifest, executable path and protocol version.
2. The NPU page or scheduling API requests an exclusive lease.
3. Core creates an independent Unix socket path, sets `RKSERVE_PLUGIN_DIR` and starts the worker.
4. The worker initializes the runtime with the granted mask and plugin-local models.
5. The worker reports ready; Core begins executing its jobs.

Workers cannot change the mask or start undeclared model contexts.

## Execution and cancellation

Admission is bounded by `max_concurrency + queue_size`; overflow returns `429`. A job enters `running` when it gets an execution slot. Queued jobs cancel immediately. If a native NPU call cannot be safely interrupted, a running job enters `canceling` and discards its result after the call returns.

Core uses unary `ExecuteJob` over gRPC/Unix domain sockets. This is an internal primitive for asynchronous jobs, not a public synchronous inference API.

## Lifecycle

Public states reflect observable behavior:

```text
installed -> ready
                |
             backoff
                |
             failed
```

Finer startup, loading and draining steps are recorded as structured events rather than transient frontend states.
