# RKServe architecture

English | [简体中文](architecture.zh-CN.md)

## Boundaries

RKServe separates the control plane from the data plane:

- `rkserve-core` is the only HTTP control plane: plugin discovery, NPU scheduling, asynchronous jobs, events, worker supervision and console static files.
- Plugin workers are the data plane. They link Rockchip runtime libraries, own model contexts, and perform preprocessing, NPU inference and postprocessing.
- The web console is an ordinary API client. Core authenticates every `/api/v1/**` request with an API key; hiding an operation in the UI is not authorization.

```text
External clients / Web Console
              |
        HTTPS / Bearer key
              |
           Caddy
              |
       loopback HTTP
              |
       +------v-------+
       | rkserve-core |
       | API / policy |
       | Job manager  |
       | NPU scheduler|
       | supervisor   |
       +------+-------+
              |
       gRPC over UDS
       +------+------+------+
       |             |      |
  YOLO worker   TTS worker  ...
       |             |
       +------v------+
      librknnrt / RGA
              |
    /dev/dri/renderD129
```

Core does not load plugin shared libraries or allow plugins to inject HTTP handlers or frontend JavaScript. A manifest declares bounded capabilities exposed under `/api/v1/plugins/{plugin_id}`.

## Plugin packages

Plugin sources live in the repository's `plugins/` directory, separate from Core. The GitHub Actions `Plugins` workflow converts each plugin's models from pinned upstream sources, builds its worker and publishes one self-contained package per plugin as a GitHub Release asset. An installed plugin does not depend on anything outside its own directory:

```text
<plugin-root>/<plugin-id>/
  plugin.toml
  bin/worker            # Release worker, RPATH $ORIGIN/../lib
  lib/                  # Private runtime libraries
  assets/               # Models and data files
  licenses/             # Third-party licenses, sources and build information
```

See [plugins/README.md](../../plugins/README.md) for the source layout and the build, conversion and installation scripts.

## Failure domains and recovery

Workers have separate PIDs, Unix sockets, working directories and environments. When a worker exits, Core temporarily retains the lease and restarts with exponential backoff of 1, 2, 4, 8 and 16 seconds, keeping the original cores. Core checks worker health every 5 seconds; repeated failures terminate the worker and enter recovery.

Repeated recovery failures trip a circuit breaker: Core disables the desired running state and releases the lease, preventing endless restarts or permanent core occupation.

## State, jobs and events

- `state.json` stores desired running state and core bindings, not PIDs, sockets or lease IDs.
- `rkserve.sqlite3` stores structured events and job metadata using WAL.
- Job input/result files are persisted using temporary files and atomic rename.
- After a Core restart, completed results remain readable; unfinished jobs are marked interrupted/failed rather than automatically replaying NPU work.
- Events can be queried by cursor, time, severity, type, plugin, lease and job.

## NPU resource model

RK3576 exposes one NPU device and two schedulable cores. The scheduler uses bitmasks:

- `core0 = 0b01`
- `core1 = 0b10`
- `core0_1 = 0b11`
- `auto` means RKServe selects the mask; the decision is not delegated to workers.

Only exclusive leases are implemented. Exclusivity is a fixed Core invariant, not a repeated optional manifest setting.

## Scheduling invariants

1. A worker cannot initialize the NPU runtime before obtaining a lease.
2. Dual-core leases are allocated atomically.
3. Rebinding drains requests and recreates the RKNN context.
4. Queues are bounded; overload is explicitly rejected rather than consuming unbounded memory.
5. Uninstalled, stopped or unhealthy plugins cannot execute jobs.
6. Workers use only the granted core mask and cannot expand their leases.

## API boundary

System APIs use `/api/v1`. Plugin capabilities expose asynchronous jobs only:

```text
POST   /api/v1/plugins/{plugin_id}/jobs/{capability_id}
GET    /api/v1/plugins/{plugin_id}/jobs/{job_id}
DELETE /api/v1/plugins/{plugin_id}/jobs/{job_id}
GET    /api/v1/plugins/{plugin_id}/jobs/{job_id}/result
GET    /api/v1/plugins/{plugin_id}/spec
```

There are no arbitrary manifest-defined public paths or synchronous `/infer` APIs. Core supplies capability, field and presentation metadata for console forms and API documentation. The internal worker protocol is separately versioned and is not exposed publicly.

Core is the final authorization point, independent of Caddy or the console. Keys receive least-privilege `read`, `infer`, `control` and `audit` scopes. There are no anonymous API endpoints except health checks. Caddy provides HTTPS/reverse proxying; Core defaults to loopback and does not trust forwarding headers by default.

## Language boundary

Core publishes canonical English display metadata and optional `translations` maps. The console selects localized display strings, never translated IDs or configuration values. English is the console default, with an explicitly saved `zh-CN` choice. Diagnostics and event payloads remain in English so logs and API responses are stable across clients; user/model output is preserved verbatim.
