# HTTP API reference

English | [简体中文](api-reference.zh-CN.md)

RKServe defaults to `127.0.0.1:8080`. Except for `/health`, endpoints live under `/api/v1`. Caddy provides HTTPS in production, but Core still authenticates every `/api/v1/**` request. The console has no special trust.

## Conventions

- Send `Authorization: Bearer <API key>` or `X-API-Key: <API key>`. Bearer takes precedence when both are present. Keys belong in headers, never URLs.
- Keys have either the `management` (console) or `api` (inference) role. An authenticated inference key still receives `403` for console read, scheduling and audit APIs.
- JSON endpoints use `application/json`.
- Job bodies are raw binary or UTF-8 text; `Content-Type` must match the capability.
- Both Core's global limit and the capability's `max_input_bytes` constrain request size.
- Responses include `X-Request-ID`. Clients may provide a safe 1–64 character request ID for end-to-end correlation.
- Errors use `{ "error": "Error description" }`; authentication errors also include `request_id`.
- API identifiers, configuration values, diagnostics and event messages are canonical, independent of the console language. Plugin summaries may include `translations` for display localization; see the [plugin contract](plugin-contract.md).

| Scope | Endpoints |
| --- | --- |
| `read` | System, telemetry, topology, plugin, worker and lease queries |
| `infer` | Job submission, status, cancellation and results |
| `control` | Plugin configuration and lease creation/release |
| `audit` | Event logs |
| `*` | All scopes |

| Status | Meaning |
| --- | --- |
| `401` | Missing or unrecognized API key |
| `403` | Insufficient scope or disallowed key role |
| `400` | Invalid request parameters |
| `404` | Plugin, capability, lease or job not found |
| `409` | Busy core, conflicting plugin state or result not ready |
| `413` | Request body exceeds the capability limit |
| `415` | Unsupported `Content-Type` |
| `422` | Core mask, manifest or worker capability mismatch |
| `429` | Job queue full |
| `500` | State, database or result persistence failed |
| `503` | Worker startup, communication, execution or shutdown failed |

## System and plugins

### `GET /health`

```json
{ "status": "ok" }
```

The only anonymous API endpoint, used by Caddy and container health checks.

### `GET /api/v1/auth/whoami`

Returns the current key's identity name, scopes and irreversible fingerprint. The console uses it to validate entered keys.

### `GET /api/v1/system`

Returns the service version, platform and number of discovered plugins.

### `GET /api/v1/system/telemetry`

Returns SoC/NPU temperature, total/available memory and per-core NPU load when exposed by the driver. Temperature zones are matched by `type`, not fixed indices. Per-core load uses an optional debugfs node. Unreadable fields return `null`, not an endpoint failure.

### `GET /api/v1/npu/topology`

Returns RK3576 NPU topology, frequency, load, cores and current leases.

### `GET /api/v1/plugins`

Returns plugin manifest summaries, capabilities, configuration fields/resolved values, runtime state and optional display translations.

### `GET /api/v1/plugins/{plugin_id}`

Returns the full summary of one plugin.

### `GET /api/v1/plugins/{plugin_id}/spec`

Returns manifest-v2-derived asynchronous API definitions, input fields, output types and examples. The console API page consumes the same data.

### `GET /api/v1/workers`

Returns worker snapshots. Worker state is `ready` or `backoff`.

## NPU leases

### `GET /api/v1/scheduler/allocations`

Returns current leases.

### `POST /api/v1/scheduler/allocations`

```json
{
  "plugin_id": "yolo26",
  "core_mask": "core0_1"
}
```

`core_mask` accepts `auto`, `core0`, `core1` or `core0_1`. Returns `201` with the created lease.

### `DELETE /api/v1/scheduler/allocations/{lease_id}`

Stops the worker and releases its lease. Returns `204` on success.

## Plugin configuration

### `PUT /api/v1/plugins/{plugin_id}/configuration`

For stopped plugins, values are saved for the next start. For running plugins, Core drains/unloads the current worker and reloads it with the same core mask and new configuration. Model/library paths cannot escape the plugin directory; model-discovery plugins accept only relative paths already found under their own `assets/models/`.

```json
{
  "values": {
    "model": "custom_people_640/custom-people-rk3576.rknn"
  }
}
```

Returns the full configuration object with defaults resolved.

## Asynchronous jobs

RKServe exposes no public synchronous inference or manifest-defined public paths. All capabilities use the Job API.

### `POST /api/v1/plugins/{plugin_id}/jobs/{capability_id}`

Query parameters become capability parameters. Body and `Content-Type` follow the manifest. Returns `202` with a job snapshot.

```bash
curl -X POST \
  'http://127.0.0.1:8080/api/v1/plugins/yolo26/jobs/vision.detect' \
  -H "X-API-Key: ${RKSERVE_API_KEY}" \
  -H 'Content-Type: image/jpeg' \
  --data-binary '@image.jpg'
```

Job states: `queued`, `running`, `canceling`, `succeeded`, `failed`, `canceled`.

### `GET /api/v1/plugins/{plugin_id}/jobs/{job_id}`

Returns state, timestamps, cancellation status, result size, stage timings and errors. The path's `plugin_id` must match the job's plugin.

### `DELETE /api/v1/plugins/{plugin_id}/jobs/{job_id}`

Requests cancellation. Queued jobs cancel immediately. Uninterruptible native NPU calls enter `canceling` and discard their result when execution safely ends.

### `GET /api/v1/plugins/{plugin_id}/jobs/{job_id}/result`

Returns the result only for `succeeded` jobs; otherwise `409`. The response uses the capability's `Content-Type` and includes available stage-timing headers.

## Structured events

### `GET /api/v1/events`

Events are paginated in descending order:

```json
{
  "items": [],
  "next_cursor": null
}
```

| Parameter | Meaning |
| --- | --- |
| `cursor` | Next-page cursor |
| `after` | Events newer than the supplied event ID |
| `limit` | Page size |
| `severity` | `info`, `warning` or `error` |
| `category` | Event category |
| `kind` | Event kind |
| `plugin_id` | Plugin ID |
| `lease_id` | Lease ID |
| `job_id` | Job ID |
| `request_id` | HTTP request ID |
| `actor` | API key identity name |
| `client_ip` | Resolved client IP |
| `http_method` | HTTP method |
| `http_status` | HTTP status code |
| `source` | Source; use `http` for HTTP audit events |
| `exclude_source` | Exclude a source; use `http` to view runtime events |
| `q` | Search message, path, request ID or actor; at most 64 characters |
| `outcome` | `success` or `failure` |
| `path_prefix` | Path prefix, such as `/api/v1/plugins` |
| `from` / `to` | Unix millisecond time range |

Events include plugin/lease/job/request IDs, identity, client/peer IP, HTTP method/path/status, duration, request/response bytes, source and structured metadata. The console uses `after` for incremental refresh and `cursor` for history.

Core does not store API keys, query parameter values or raw request/response content in events. Job events correlate inputs/outputs with SHA-256. Failures, authentication failures, state changes, whoami, plugin specs and result downloads are persisted in the HTTP audit log (`source=http`); successful high-frequency console polling is not logged row by row. Defaults are 30 days and 100,000 rows, configurable through `RKSERVE_EVENT_RETENTION_DAYS` and `RKSERVE_EVENT_MAX_ROWS`.
