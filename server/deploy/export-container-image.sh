#!/usr/bin/env bash
set -euo pipefail

if [[ "$(id -u)" == "0" ]]; then
  echo "Error: run as the regular user who built the image, without sudo." >&2
  exit 1
fi
if ! command -v podman >/dev/null 2>&1; then
  echo "Error: podman was not found." >&2
  exit 1
fi

image="${1:-localhost/rkserve:latest}"
output="${2:-${PWD}/rkserve-image.tar}"
output_dir="$(dirname "$output")"
output_name="$(basename "$output")"

if [[ ! -d "$output_dir" ]]; then
  echo "Error: output directory does not exist: ${output_dir}" >&2
  exit 1
fi
if [[ -e "$output" || -e "${output}.sha256" ]]; then
  echo "Error: output already exists; refusing to overwrite: ${output}" >&2
  exit 1
fi
if ! podman image exists "$image"; then
  echo "Error: image not found in this user's rootless Podman storage: ${image}" >&2
  exit 1
fi

cleanup_partial() {
  if [[ "${export_complete:-false}" != "true" ]]; then
    rm -f -- "$output" "${output}.sha256"
  fi
}
trap cleanup_partial EXIT

umask 022
echo "Exporting ${image} to ${output}"
podman save --format docker-archive --output "$output" "$image"
(
  cd "$output_dir"
  sha256sum "$output_name" > "${output_name}.sha256"
)
chmod 0644 "$output" "${output}.sha256"
export_complete=true

echo "Export complete:"
ls -lh "$output" "${output}.sha256"
cat "${output}.sha256"
