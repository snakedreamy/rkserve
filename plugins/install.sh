#!/usr/bin/env bash
# Install RKServe plugins from a GitHub release (or a local directory).
# Usage: plugins/install.sh [--dest DIR] [--release TAG | --from DIR] <plugin-id>...
set -euo pipefail

repo="${RKSERVE_REPOSITORY:-snakedreamy/rkserve}"
release="${RKSERVE_PLUGINS_RELEASE:-plugins-v0.1.0}"
dest="${RKSERVE_PLUGIN_ROOT:-}"
from=""
plugins=()

usage() {
  cat <<EOF
Usage: $0 [--dest DIR] [--release TAG | --from DIR] <plugin-id>...

  --dest DIR     plugin root used by RKServe (default: \$RKSERVE_PLUGIN_ROOT)
  --release TAG  GitHub release to download from (default: ${release})
  --from DIR     install from rkserve-plugin-*.tar.gz and SHA256SUMS or *.tar.gz.sha256 in DIR

Plugins: matcha-tts sensevoice-asr zipformer-asr yolo26 yolov8-pose
Restart RKServe after installing or updating plugins.
EOF
}

while (($# > 0)); do
  case "$1" in
    --dest) dest="${2:?--dest needs a directory}"; shift 2 ;;
    --release) release="${2:?--release needs a tag}"; shift 2 ;;
    --from) from="${2:?--from needs a directory}"; shift 2 ;;
    -h | --help) usage; exit 0 ;;
    -*) echo "Error: unknown option $1" >&2; usage >&2; exit 2 ;;
    *) plugins+=("$1"); shift ;;
  esac
done
if [[ -z "$dest" || ${#plugins[@]} -eq 0 ]]; then
  usage >&2
  exit 2
fi
for tool in curl sha256sum tar python3 flock; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Error: ${tool} was not found." >&2
    exit 1
  fi
done

# Do not install the same plugin twice in one run; that would overwrite the rollback copy.
for ((i = 0; i < ${#plugins[@]}; i++)); do
  for ((j = 0; j < i; j++)); do
    if [[ "${plugins[i]}" == "${plugins[j]}" ]]; then
      echo "Error: duplicate plugin id: ${plugins[i]}" >&2
      exit 2
    fi
  done
done
mkdir -p "$dest"
# Serialize installers so two processes cannot replace the same plugin; keep the lock file to avoid inode races.
exec 9>"${dest%/}/.install.lock"
flock -n 9 || { echo "Error: another installer is running." >&2; exit 1; }
work_dir="$(mktemp -d "${dest%/}/.install.XXXXXX")"
cleanup() {
  local old id
  for old in "$work_dir"/*.old; do
    [[ -e "$old" ]] || continue
    id="$(basename "$old" .old)"
    if [[ ! -e "${dest}/${id}" ]]; then
      mv -- "$old" "${dest}/${id}" || return 1
    fi
  done
  rm -rf -- "$work_dir"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

fetch() {
  if [[ -n "$from" ]]; then
    cp -- "${from%/}/$1" "${work_dir}/$1"
  else
    curl --fail --location --retry 3 --silent --show-error --output "${work_dir}/$1" \
      "https://github.com/${repo}/releases/download/${release}/$1"
  fi
}

if [[ -n "$from" ]]; then
  if [[ -f "${from%/}/SHA256SUMS" ]]; then
    fetch SHA256SUMS
  fi
else
  fetch SHA256SUMS
fi
for plugin in "${plugins[@]}"; do
  if [[ ! "$plugin" =~ ^[a-z0-9][a-z0-9-]*$ ]]; then
    echo "Error: invalid plugin id: ${plugin}" >&2
    exit 1
  fi
  archive="rkserve-plugin-${plugin}.tar.gz"
  checksum=""
  if [[ -f "${work_dir}/SHA256SUMS" ]]; then
    checksum="$(awk -v file="$archive" '$2 == file { print $1 }' "${work_dir}/SHA256SUMS")"
  fi
  sidecar="${archive}.sha256"
  if [[ ! "$checksum" =~ ^[0-9a-f]{64}$ && -n "$from" && -f "${from%/}/${sidecar}" ]]; then
    fetch "$sidecar"
    checksum="$(awk -v file="$archive" '$2 == file { print $1 }' "${work_dir}/${sidecar}")"
  fi
  if [[ ! "$checksum" =~ ^[0-9a-f]{64}$ ]]; then
    echo "Error: ${archive} is not listed in SHA256SUMS of ${from:-release ${release}}." >&2
    exit 1
  fi
  echo "${plugin}: downloading ${archive}"
  fetch "$archive"
  (cd "$work_dir" && printf '%s  %s\n' "$checksum" "$archive" | sha256sum --quiet --check -)

  staging="${work_dir}/${plugin}.new"
  mkdir -p "$staging"
  # Accept only regular files and directories inside the plugin directory; reject links, devices and path traversal.
  python3 - "${work_dir}/${archive}" "$plugin" "$staging" <<'PYTHON'
import pathlib
import sys
import tarfile

archive, plugin, destination = sys.argv[1:]
with tarfile.open(archive, "r:gz") as bundle:
    members = bundle.getmembers()
    seen = set()
    for member in members:
        path = pathlib.PurePosixPath(member.name)
        if (path.is_absolute() or ".." in path.parts or not path.parts
                or path.parts[0] != plugin or not (member.isfile() or member.isdir())
                or str(path) in seen):
            raise SystemExit(f"refusing unsafe archive member: {member.name}")
        seen.add(str(path))
        member.mode &= 0o777
    bundle.extractall(destination, members=members)
PYTHON
  if [[ ! -f "${staging}/${plugin}/plugin.toml" ]]; then
    echo "Error: ${archive} does not contain ${plugin}/plugin.toml." >&2
    exit 1
  fi
  python3 - "$plugin" "${staging}/${plugin}" <<'PYTHON'
import pathlib
import re
import sys

plugin, root = sys.argv[1], pathlib.Path(sys.argv[2])
text = (root / "plugin.toml").read_text(encoding="utf-8")
try:
    import tomllib
    data = tomllib.loads(text)
except ImportError:
    data = None
except Exception as error:
    raise SystemExit(f"invalid plugin.toml: {error}") from error

def fail(message):
    raise SystemExit(message)

if data is None:
    if not re.search(r"(?m)^schema_version\s*=\s*2\s*$", text):
        fail("unsupported manifest schema")
    if not re.search(r'(?m)^id\s*=\s*"%s"\s*$' % re.escape(plugin), text):
        fail("plugin id does not match the archive")
    if not re.search(r"(?m)^protocol_version\s*=\s*2\s*$", text):
        fail("unsupported plugin protocol")
    if not re.search(r'(?m)^executable\s*=\s*"bin/worker"\s*$', text):
        fail("plugin executable must be bin/worker")
else:
    if data.get("schema_version") != 2:
        fail("unsupported manifest schema")
    section = data.get("plugin") or {}
    if section.get("id") != plugin:
        fail("plugin id does not match the archive")
    if section.get("protocol_version") != 2:
        fail("unsupported plugin protocol")
    executable = section.get("executable") or ""
    path = pathlib.PurePosixPath(executable)
    if path.is_absolute() or ".." in path.parts or not path.parts:
        fail("plugin executable path is invalid")
    target = root / path
    if not target.is_file() or target.is_symlink():
        fail("plugin executable is missing")
    npu = (data.get("resources") or {}).get("npu") or {}
    allowed = npu.get("allowed_masks") or []
    if not allowed or npu.get("default_mask") not in allowed:
        fail("invalid NPU core mask configuration")
    for key in ("max_concurrency", "queue_size", "request_timeout_ms"):
        if not isinstance(npu.get(key), int) or npu.get(key) <= 0:
            fail(f"{key} must be a positive integer")
    if not data.get("capabilities"):
        fail("plugin must declare at least one capability")
PYTHON
  if [[ ! -x "${staging}/${plugin}/bin/worker" ]]; then
    echo "Error: ${archive} has no executable bin/worker." >&2
    exit 1
  fi
  if [[ -L "${dest}/${plugin}" ]]; then
    echo "Error: refusing to replace a symlink: ${dest}/${plugin}" >&2
    exit 1
  fi
  if [[ -e "${dest}/${plugin}" ]]; then
    mv -- "${dest}/${plugin}" "${work_dir}/${plugin}.old"
  fi
  if ! mv -- "${staging}/${plugin}" "${dest}/${plugin}"; then
    if [[ -e "${work_dir}/${plugin}.old" ]]; then
      mv -- "${work_dir}/${plugin}.old" "${dest}/${plugin}"
    fi
    exit 1
  fi
  echo "${plugin}: installed to ${dest%/}/${plugin}"
done
echo "Restart RKServe to load the installed plugins."
