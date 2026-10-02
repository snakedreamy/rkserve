# Security and Caddy deployment

English | [简体中文](security-and-deployment.zh-CN.md)

## Trust boundaries

- `/health` and console static files are public. `rkserve-core` authenticates all `/api/v1/**` requests, including console requests, using `Authorization: Bearer <key>` or `X-API-Key: <key>`. Bearer takes precedence.
- Caddy terminates TLS but is not the authorization boundary. Direct requests bypassing the console must still pass Core authentication.
- Core defaults to `127.0.0.1:8080`. Do not expose its port to the LAN or internet behind Caddy.
- RKServe ignores `X-Forwarded-For` unless the direct peer is listed in `RKSERVE_TRUSTED_PROXIES`. It then uses the first forwarded address only for auditing, never authorization.

## Create API keys

From the repository root, copy the example and generate two separate keys: a management key for the console and an API key for inference jobs.

```bash
install -m 600 server/deploy/api-keys.example.json server/deploy/api-keys.json
RKSERVE_KEY="$(openssl rand -hex 32)"
RKSERVE_API_KEY="$(openssl rand -hex 32)"
sed -i "s/REPLACE_WITH_A_RANDOM_KEY_OF_AT_LEAST_32_BYTES/${RKSERVE_KEY}/" server/deploy/api-keys.json
sed -i "s/REPLACE_WITH_A_SECOND_RANDOM_KEY_OF_AT_LEAST_32_BYTES/${RKSERVE_API_KEY}/" server/deploy/api-keys.json
printf 'Console management key: %s\n' "${RKSERVE_KEY}"
printf 'Inference API key: %s\n' "${RKSERVE_API_KEY}"
unset RKSERVE_KEY RKSERVE_API_KEY
```

The real `server/deploy/api-keys.json` is ignored by Git. Core rejects missing files, symlinks, group/other access, keys shorter than 32 bytes, placeholders, duplicate names/keys, unknown scopes, role/scope mismatches, and configurations with no enabled keys. Set `"enabled": false` to disable a key without deleting it; disabled entries still undergo format and uniqueness checks. Restart Core after changing the key file; keys are loaded at startup.

Key roles:

| Role | Purpose |
|---|---|
| `management` | Management key for the console, system, plugins, scheduling and auditing. Add `infer` to run inference from the console. |
| `api` | Inference key. Only `/api/v1/auth/whoami` and plugin job endpoints are allowed; no console sign-in, audit access or scheduling changes. |

Available scopes:

| Scope | Capabilities |
|---|---|
| `read` | Read-only system, NPU, plugin, worker and lease queries |
| `infer` | Submit, query and cancel jobs; download results |
| `control` | Change plugin configuration; create and release NPU leases |
| `audit` | Query events and audit logs |
| `*` | All capabilities; only recommended for controlled management keys |

## Install plugins

Core ships without plugins. Install the plugins you need from the plugin release into a plugin root directory; each package is verified against the release `SHA256SUMS`:

```bash
plugins/install.sh --dest "$HOME/rkserve/plugins" yolo26 sensevoice-asr
```

Run the same command again to update a plugin, then restart Core. See [plugins/README.md](../../plugins/README.md) for the available plugins and their licenses.

## Start RKServe

```bash
RKSERVE_LISTEN=127.0.0.1:8080 \
RKSERVE_API_KEYS_FILE="$PWD/server/deploy/api-keys.json" \
RKSERVE_TRUSTED_PROXIES=127.0.0.1,::1 \
RKSERVE_EVENT_RETENTION_DAYS=30 \
RKSERVE_EVENT_MAX_ROWS=100000 \
RKSERVE_PLUGIN_ROOT="$HOME/rkserve/plugins" \
RKSERVE_RUNTIME_ROOT=run/plugins \
RKSERVE_STATE_ROOT=run \
RKSERVE_CONSOLE_ROOT=server/frontend/dist \
  server/target/release/rkserve-core
```

If both Caddy and RKServe are containerized, place them in one Podman pod and keep using `127.0.0.1:8080` in the shared network namespace. Do not map Core to host `0.0.0.0:8080` for convenience.

## Build with rootless Podman on ARM

`server/Containerfile` is built from the repository root and has three stages:

1. A Node builder generates console static files.
2. A Rust builder compiles Core.
3. The Ubuntu 24.04 runtime copies only Core, the console and the project license.

The image contains no plugins, models or RKNN libraries; plugins are installed on the host and mounted read-only at `/opt/rkserve/plugins`. Worker builds target Debian 11; the minimum runtime glibc requirement of the complete package, including downloaded libraries, still needs validation. Rust, Node, GCC and JavaScript package managers are neither installed on the host nor included in the runtime image. Build caches and base images stay in the current user's Podman storage. The frontend pins Node 20.20.2 and pnpm 10.34.5 for Vite 8, with `pnpm-lock.yaml`; this avoids observed npm failures that returned success in ARM containers.

Host requirements:

- aarch64 Armbian.
- Rootless Podman, preferably using the `crun` OCI runtime.
- Python 3, `curl`, `tar` and `sha256sum` to download and verify plugin packages.
- The regular user belongs to the `render` and `video` supplementary groups.

An administrator may need to configure device groups once:

```bash
sudo usermod -aG render,video "$USER"
```

Log out and back in; confirm `id -nG` includes `render` and `video`. Do not use `sudo` for builds or routine operation.

Build:

```bash
./server/deploy/build-container.sh localhost/rkserve:latest
```

The script rejects root users, non-aarch64 hosts and non-rootless Podman. Reduce parallelism on memory-constrained devices:

```bash
RKSERVE_BUILD_JOBS=2 ./server/deploy/build-container.sh localhost/rkserve:latest
```

Podman forwards host proxy variables such as `http_proxy` and `https_proxy` to builders. For proxies on `127.0.0.1`, `localhost` or `[::1]`, the script selects host networking for build `RUN` steps only. This increases local network visibility during builds but does not persist in the image or change runtime networking. Frontend dependency lifecycle scripts are disabled. Otherwise Podman uses isolated networking. Set `RKSERVE_BUILD_NETWORK=private` to prevent automatic switching, or explicitly choose another supported build network mode.

The root `.dockerignore` (also read by Podman) admits only Core, protocol, console sources and license files into the build context; build outputs, `node_modules`, plugins and Git data stay out.

The build uses Docker v2 image format to retain health-check metadata. It does not change host software, but images and builder layers consume the user's container storage. Inspect usage with `podman system df`; do not globally prune other projects' caches without understanding the effects.

## Create a Podman secret

Write a random key directly to a rootless Podman secret, without a plaintext JSON file:

```bash
RKSERVE_KEY="$(openssl rand -hex 32)"
printf '{"keys":[{"name":"console","key":"%s","scopes":["read","infer","control","audit"]}]}\n' \
  "${RKSERVE_KEY}" |
  podman secret create rkserve-api-keys -
printf 'Save this console API key in a password manager now: %s\n' "${RKSERVE_KEY}"
unset RKSERVE_KEY
```

The key is not embedded in the image, Git or container environment. At runtime it is mounted at `/run/secrets/rkserve-api-keys` with UID/GID 0 and mode `0400`.

## Run as container root under rootless Podman

The example uses a local `localhost/rkserve:latest` image. Once GitHub Actions has published an image, run `podman pull ghcr.io/snakedreamy/rkserve:latest` and substitute that image name.

For host Caddy, this example uses host networking but listens only on loopback. The [Quadlet guide](../deploy/quadlet/README.md) provides an isolated-network alternative. First create a persistent volume and install plugins into a host directory:

```bash
podman volume create rkserve-state
plugins/install.sh --dest "$HOME/rkserve/plugins" yolo26
```

Start as a regular user:

```bash
podman run -d \
  --name rkserve \
  --network host \
  --init \
  --read-only \
  --tmpfs /run/rkserve:rw,nosuid,nodev,noexec,size=64m,mode=0700 \
  --tmpfs /tmp:rw,nosuid,nodev,noexec,size=64m,mode=1777 \
  --volume rkserve-state:/var/lib/rkserve:rw \
  --volume "$HOME/rkserve/plugins:/opt/rkserve/plugins:ro" \
  --secret rkserve-api-keys,type=mount,target=/run/secrets/rkserve-api-keys,uid=0,gid=0,mode=0400 \
  --device /dev/dri/renderD129 \
  --device /dev/rga \
  --device /dev/dma_heap/system \
  --device /dev/dma_heap/system-uncached \
  --group-add keep-groups \
  --cap-drop all \
  --security-opt no-new-privileges \
  --pids-limit 256 \
  --env RKSERVE_LISTEN=127.0.0.1:8080 \
  --env RKSERVE_TRUSTED_PROXIES=127.0.0.1,::1 \
  localhost/rkserve:latest
```

Omit `/dev/dma_heap/*` mappings for nodes absent on your device. `renderD129` is required for the RK3576 NPU; `/dev/rga` accelerates image preprocessing.

Container UID 0 maps to the regular host user, not host root. `--group-add keep-groups` preserves `render`/`video` groups for device DAC checks and requires `crun`. All capabilities are dropped, the root filesystem is read-only, and `no-new-privileges` is enabled. Do not use `--privileged`.

If SELinux denies device access, ask an administrator to configure `container_use_devices` according to Podman documentation, rather than using `--privileged`. AppArmor-based Armbian/Ubuntu systems generally do not need this SELinux setting.

Host Caddy proxies `127.0.0.1:8080`. Do not expose RKServe port 8080 to the LAN. If Caddy is later containerized, use one shared pod and expose only Caddy HTTP/HTTPS ports.

NPU inference does not require debugfs. Rootless containers usually cannot read `/sys/kernel/debug/rknpu/load`, so instantaneous per-core load may be unavailable. Model loading, inference and overall devfreq metrics are unaffected. Do not mount all of debugfs or use privileged containers just for monitoring.

Routine checks:

```bash
podman healthcheck run rkserve
podman logs rkserve
curl http://127.0.0.1:8080/health
```

## Caddy

Copy [`deploy/Caddyfile.example`](../deploy/Caddyfile.example) into your Caddy configuration and replace the domain. The example:

- Obtains and renews HTTPS certificates.
- Overwrites forwarding headers to prevent client-supplied spoofing.
- Caps request bodies at 64 MiB, matching Core.
- Adds HSTS, CSP, anti-MIME-sniffing and anti-framing headers.

Caddy forwards `Authorization` and `X-API-Key`; RKServe performs final authentication. Never record these headers in Caddy access logs.

## Console key handling

The console has no username/password login and issues no cookies. Enter the API key manually; it is stored in tab-scoped `sessionStorage`, surviving refresh but cleared when the session ends. Never put keys in frontend bundles, URL parameters, Caddyfiles or Git. The language preference is non-secret and is stored separately in `localStorage`.

## Audit data

Audit logs record identity, key fingerprint, client/peer IP, request ID, method, path, status, duration, request/response size, media type and plugin/job associations. Jobs also record input/output SHA-256.

Audit events do not store API keys, query values, original text/images/audio or inference results. Successful high-frequency read polling (topology, telemetry, workers, allocations, plugins, events) is not persisted individually. Failures, authentication failures, whoami, plugin specs, state changes, job submissions and result downloads are persisted with `source=http`. The console separates API audit events from scheduling/worker runtime events. Retention defaults to 30 days and 100,000 rows, whichever limit is reached first. Diagnostic messages remain English in both console languages.
