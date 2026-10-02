# RKServe

[![CI](https://github.com/snakedreamy/rkserve/actions/workflows/ci.yml/badge.svg)](https://github.com/snakedreamy/rkserve/actions/workflows/ci.yml)
[![Image](https://github.com/snakedreamy/rkserve/actions/workflows/image.yml/badge.svg)](https://github.com/snakedreamy/rkserve/pkgs/container/rkserve)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

English | [简体中文](README.zh-CN.md)

RKServe is an NPU serving platform for Rockchip RK35xx edge devices. `rkserve-core` handles NPU scheduling, plugin lifecycle, asynchronous jobs, structured events and a single HTTP boundary. Algorithms run as separate worker processes, keeping crashes and native dependencies out of Core.

## Design principles

- Core never links `librknnrt`; each worker owns its model context.
- Plugins explicitly declare allowed cores, concurrency, queue size and timeouts.
- Each installed plugin is self-contained: its worker, models, runtime libraries and licenses live in its own directory.
- Inference is exposed only as asynchronous jobs, not synchronous `/infer` or arbitrary public endpoints.
- Core is the only HTTP trust boundary. Plugins cannot inject server routes or frontend JavaScript.
- Every `/api/v1/**` request requires API key authentication; the console is an ordinary, unprivileged API client.

## Languages

The console defaults to **English**, with a **简体中文** switch available before and after sign-in. The choice is saved locally; browser language does not override the default. Plugin descriptions, form labels, statuses and date/number formatting follow the selected language. Missing translations fall back to English.

Project documentation has matching `.zh-CN.md` versions. Maintained code comments, server diagnostics and script output use English. API identifiers, model paths, user input, recognition results, model vocabularies and third-party license texts are not translated.

## Plugins

Core ships without plugins. Install only the ones you need from the GitHub Release. Models are converted for **RK3576**; other RK35xx chips need their own conversion.

| Plugin | Capability | Model |
| --- | --- | --- |
| `yolo26` | Object detection | Ultralytics YOLO26 n/s/m/l/x (FP16, 640) |
| `yolov8-pose` | Human pose estimation | YOLOv8n-Pose |
| `sensevoice-asr` | Multilingual speech understanding (text, language, emotion, events) | SenseVoice-small |
| `zipformer-asr` | Chinese/English speech recognition | Zipformer bilingual zh-en |
| `matcha-tts` | Chinese/English speech synthesis | Matcha-TTS + Vocos |

```bash
plugins/install.sh --dest "$HOME/rkserve/plugins" yolo26 sensevoice-asr
```

The installer downloads `rkserve-plugin-*.tar.gz` from GitHub Releases and verifies SHA-256. Use `--from DIR` to install from local archives instead. Models and bundled third-party components keep their own licenses (including AGPL-3.0, GPL-3.0 and the Rockchip RKNN SDK license); see [plugins/README.md](plugins/README.md).

## Repository layout

```text
server/                 RKServe itself (Apache-2.0)
  core/                 HTTP API, scheduling, jobs, events and worker supervision
  protocol/             Core/worker Protobuf protocol
  frontend/             React console
  deploy/               Container build, Quadlet, Caddy and API key examples
  docs/                 Architecture, plugin contract, HTTP API and deployment
  Containerfile         Core + console image, without plugins

plugins/                Plugin sources, model conversion and packaging
  <plugin-id>/          Manifest, worker, conversion scripts and licenses
  tools/                Shared build, dependency and packaging scripts
  deps.lock             Official URLs and SHA-256 of downloaded third-party files
  install.sh            Plugin installer
```

Model weights, compiled workers and downloaded runtime libraries are not committed to Git; small data assets and console fonts remain tracked.

## Quick start: image and plugins

The published `linux/arm64` image contains Core and the console. Plugins are installed on the host and mounted into the container.

```bash
podman pull ghcr.io/snakedreamy/rkserve:latest
plugins/install.sh --dest "$HOME/rkserve/plugins" yolo26
```

Create an API key secret, map the NPU and RGA devices, and mount the plugin directory at `/opt/rkserve/plugins`. See [Security and Caddy deployment](server/docs/security-and-deployment.md), or [Rootless Quadlet deployment](server/deploy/quadlet/README.md) for a long-running service.

## Build from source

Build Core and the console on the device (from the repository root):

```bash
(cd server && cargo build --locked --release -p rkserve-core)
(cd server/frontend && corepack enable pnpm && pnpm install --frozen-lockfile && pnpm run build)
```

Plugins are installed from the GitHub Release. To build a plugin yourself, see [plugins/README.md](plugins/README.md).

## Run

From the repository root, create the key file and start Core with the installed plugins:

```bash
install -m 600 server/deploy/api-keys.example.json server/deploy/api-keys.json
# Generate keys with openssl rand -hex 32 and replace the placeholders.

RKSERVE_LISTEN=127.0.0.1:8080 \
RKSERVE_API_KEYS_FILE=server/deploy/api-keys.json \
RKSERVE_PLUGIN_ROOT="$HOME/rkserve/plugins" \
RKSERVE_RUNTIME_ROOT=run/plugins \
RKSERVE_STATE_ROOT=run \
RKSERVE_CONSOLE_ROOT=server/frontend/dist \
  server/target/release/rkserve-core
```

Core refuses to start without a valid `RKSERVE_API_KEYS_FILE`. It defaults to `127.0.0.1:8080`; use Caddy for external HTTPS access. Runtime data is excluded by `.gitignore`:

- `RKSERVE_RUNTIME_ROOT`: Unix sockets, worker markers and short-lived files.
- `RKSERVE_STATE_ROOT/state.json`: desired plugin state and core allocations.
- `RKSERVE_STATE_ROOT/rkserve.sqlite3`: events and job metadata.
- `RKSERVE_STATE_ROOT/jobs/`: job inputs and results.

Open the configured HTTPS domain and enter the API key assigned at deployment. The YOLO26 plugin configuration lists models discovered under its `assets/models/` directory.

## Console pages

- `/npu`: NPU topology, core allocation and the only plugin start/stop controls.
- `/plugins`: read-only plugin inventory, states and capabilities.
- `/plugins/:id`: configuration, resources and runtime state; no start/stop controls.
- `/plugins/:id/run`: submit, poll and cancel asynchronous jobs and read results.
- `/apis`: manifest-driven asynchronous API documentation.
- `/events`: query runtime events by cursor and structured fields.

## Asynchronous API examples

Set your key, then start YOLO26 from the NPU page or the scheduling API:

```bash
export RKSERVE_API_KEY='<your-deployment-key>'
curl -X POST http://127.0.0.1:8080/api/v1/scheduler/allocations \
  -H "Authorization: Bearer ${RKSERVE_API_KEY}" \
  -H 'Content-Type: application/json' \
  -d '{"plugin_id":"yolo26","core_mask":"core0_1"}'
```

Submit an object-detection job:

```bash
curl -X POST \
  http://127.0.0.1:8080/api/v1/plugins/yolo26/jobs/vision.detect \
  -H "Authorization: Bearer ${RKSERVE_API_KEY}" \
  -H 'Content-Type: image/jpeg' \
  --data-binary '@image.jpg'
```

SenseVoice returns structured JSON with text, language, emotion, sound events and segments for long recordings:

```bash
curl -X POST \
  'http://127.0.0.1:8080/api/v1/plugins/sensevoice-asr/jobs/audio.transcribe?language=auto&text_normalization=withitn' \
  -H "Authorization: Bearer ${RKSERVE_API_KEY}" \
  -H 'Content-Type: audio/wav' \
  --data-binary '@speech.wav'
```

Zipformer also uses asynchronous jobs; beam size `4` prioritizes accuracy:

```bash
curl -X POST \
  'http://127.0.0.1:8080/api/v1/plugins/zipformer-asr/jobs/audio.transcribe?beam_size=4' \
  -H "Authorization: Bearer ${RKSERVE_API_KEY}" \
  -H 'Content-Type: audio/wav' \
  --data-binary '@speech.wav'
```

Use the returned `id` to query status and results (replace `JOB_ID`):

```bash
curl -H "Authorization: Bearer ${RKSERVE_API_KEY}" \
  http://127.0.0.1:8080/api/v1/plugins/yolo26/jobs/JOB_ID
curl -o result.json \
  -H "Authorization: Bearer ${RKSERVE_API_KEY}" \
  http://127.0.0.1:8080/api/v1/plugins/yolo26/jobs/JOB_ID/result
```

## Documentation

- [Architecture](server/docs/architecture.md)
- [Plugin contract](server/docs/plugin-contract.md)
- [HTTP API](server/docs/api-reference.md)
- [Security and Caddy deployment](server/docs/security-and-deployment.md)
- [Plugins: install, build and licenses](plugins/README.md)
- [Releasing](server/docs/releasing.md)

## License

The code in `server/` and the plugin sources in `plugins/` are licensed under [Apache License 2.0](LICENSE), except the `matcha-tts` worker, which is GPL-3.0-only. Plugin packages bundle third-party models and libraries under their own licenses; see [plugins/README.md](plugins/README.md).
