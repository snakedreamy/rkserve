#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$repo_root"

if [[ "$(id -u)" == "0" ]]; then
  echo "Error: run as a regular user, without sudo." >&2
  exit 1
fi
if [[ "$(uname -m)" != "aarch64" ]]; then
  echo "Error: RKServe containers must be built natively on an aarch64 ARM host." >&2
  exit 1
fi
if ! command -v podman >/dev/null 2>&1; then
  echo "Error: podman was not found; install Podman on the host." >&2
  exit 1
fi
if [[ "$(podman info --format '{{.Host.Security.Rootless}}')" != "true" ]]; then
  echo "Error: Podman is not running rootless; run directly as a regular user." >&2
  exit 1
fi

image="${1:-localhost/rkserve:latest}"
jobs="${RKSERVE_BUILD_JOBS:-4}"
revision="$(git rev-parse --verify HEAD 2>/dev/null || printf unknown)"
network_args=()

build_network="${RKSERVE_BUILD_NETWORK:-auto}"
if [[ "$build_network" == "auto" ]]; then
  for proxy_name in \
    http_proxy https_proxy ftp_proxy all_proxy \
    HTTP_PROXY HTTPS_PROXY FTP_PROXY ALL_PROXY
  do
    proxy_value="${!proxy_name:-}"
    if [[ "$proxy_value" =~ (^|://)(127\.0\.0\.1|localhost|\[::1\])(:|/|$) ]]; then
      build_network="host"
      echo "${proxy_name} uses a host loopback proxy; build RUN steps will use host networking."
      break
    fi
  done
fi
if [[ "$build_network" != "auto" && "$build_network" != "private" ]]; then
  network_args=(--network "$build_network")
fi

echo "Building ${image} with rootless Podman (parallel jobs: ${jobs})"
exec podman build \
  --format docker \
  --pull=newer \
  "${network_args[@]}" \
  --build-arg "CARGO_BUILD_JOBS=${jobs}" \
  --build-arg "RKSERVE_REVISION=${revision}" \
  --tag "$image" \
  --file server/Containerfile \
  .
