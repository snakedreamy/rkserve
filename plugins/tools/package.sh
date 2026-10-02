#!/usr/bin/env bash
# Assemble a plugin release package from the worker build and the converted models.
# Usage: plugins/tools/package.sh <plugin-id> <worker-dir> <models-dir> <output-dir>
#   worker-dir  output of plugins/<id>/build.sh (bin/, lib/, licenses/BUILD-DEPS.txt, ...)
#   models-dir  output of plugins/<id>/convert/convert.sh (assets/, licenses/MODEL-SOURCES.txt)
# Produces <output-dir>/rkserve-plugin-<id>.tar.gz with a top-level <id>/ directory.
set -euo pipefail

plugin="${1:?plugin id is required}"
worker_dir="${2:?worker directory is required}"
models_dir="${3:?models directory is required}"
output_dir="${4:?output directory is required}"

plugins_root="$(cd "$(dirname "$0")/.." && pwd)"
repo_root="$(dirname "$plugins_root")"
plugin_dir="${plugins_root}/${plugin}"
if [[ ! "$plugin" =~ ^[a-z0-9][a-z0-9-]*$ || ! -f "${plugin_dir}/plugin.toml" ]]; then
  echo "Error: unknown plugin ${plugin}." >&2
  exit 1
fi
if [[ -e "${models_dir}/licenses/CONVERSION-INCOMPLETE" ]]; then
  echo "Error: model conversion is incomplete; refusing to package." >&2
  exit 1
fi
for required in "${worker_dir}/bin/worker" "${models_dir}/assets" "${models_dir}/licenses/MODEL-SOURCES.txt" "${models_dir}/licenses/OUTPUTS.sha256" "${plugin_dir}/convert/outputs.json"; do
  if [[ ! -e "$required" ]]; then
    echo "Error: ${required} is missing." >&2
    exit 1
  fi
done
if [[ "$(head -n 1 "${models_dir}/licenses/MODEL-SOURCES.txt")" != "Plugin: ${plugin}" ]]; then
  echo "Error: model conversion output does not belong to ${plugin}." >&2
  exit 1
fi
for blocked in plugin.toml bin lib; do
  if [[ -e "${models_dir}/${blocked}" ]]; then
    echo "Error: model conversion output must not contain ${blocked}." >&2
    exit 1
  fi
done

# Failed or altered conversion output must not enter a release package.
(cd "$models_dir" && sha256sum --strict --check licenses/OUTPUTS.sha256)
work_dir="$(mktemp -d)"
archive_tmp=''
trap 'rm -rf -- "$work_dir"; [[ -z "$archive_tmp" ]] || rm -f -- "$archive_tmp"' EXIT
stage="${work_dir}/${plugin}"
mkdir -p "$stage"

install -m 0644 "${plugin_dir}/plugin.toml" "${stage}/plugin.toml"
for readme in "${plugin_dir}"/README*.md; do
  [[ -e "$readme" ]] && install -m 0644 "$readme" "${stage}/"
done
cp -R "${plugin_dir}/licenses" "${stage}/licenses"
install -m 0644 "${repo_root}/LICENSE" "${stage}/licenses/RKServe-Apache-2.0"
install -m 0644 "${repo_root}/NOTICE" "${stage}/licenses/RKServe-NOTICE"
cp -R "${worker_dir}/." "${stage}/"
# Worker output must not overwrite the repository manifest or project licenses.
install -m 0644 "${plugin_dir}/plugin.toml" "${stage}/plugin.toml"
install -m 0644 "${repo_root}/LICENSE" "${stage}/licenses/RKServe-Apache-2.0"
install -m 0644 "${repo_root}/NOTICE" "${stage}/licenses/RKServe-NOTICE"
mkdir -p "${stage}/assets" "${stage}/licenses"
cp -R "${models_dir}/assets/." "${stage}/assets/"
cp -R "${models_dir}/licenses/." "${stage}/licenses/"

python3 - "$plugin_dir" "$stage" <<'PYTHON'
import json
import pathlib
import sys

plugin_dir, stage = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
expected = json.loads((plugin_dir / "convert/outputs.json").read_text())
missing = [path for path in expected if not (stage / path).is_file() or (stage / path).is_symlink() or (stage / path).stat().st_size == 0]
if missing:
    raise SystemExit("missing conversion artifacts: " + ", ".join(missing))
PYTHON

revision="$(git -C "$repo_root" rev-parse HEAD 2>/dev/null || echo unknown)"
worker_revision="$(awk -F': ' '/^Source-Revision:/{print $2; exit}' "${worker_dir}/licenses/BUILD-DEPS.txt" 2>/dev/null || true)"
if [[ -n "${worker_revision:-}" && "$worker_revision" != unknown && "$worker_revision" != "$revision" ]]; then
  echo "Error: worker was built from ${worker_revision}, packaging ${revision}." >&2
  exit 1
fi
{
  echo "RKServe plugin ${plugin}"
  echo "Source: https://github.com/snakedreamy/rkserve/tree/${revision}/plugins/${plugin}"
  if [[ -n "${GITHUB_RUN_ID:-}" ]]; then
    echo "Build log: ${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}/actions/runs/${GITHUB_RUN_ID}"
  fi
  echo
  echo "Package contents (sha256):"
  (cd "$stage" && find . -type f ! -path ./licenses/BUILD-INFO.txt -print0 | LC_ALL=C sort -z | xargs -0 sha256sum)
} > "${work_dir}/BUILD-INFO.txt"
install -m 0644 "${work_dir}/BUILD-INFO.txt" "${stage}/licenses/BUILD-INFO.txt"

chmod -R u=rwX,go=rX "$stage"
# Artifact downloads from Actions do not keep the executable bit; restore it before packaging.
chmod 0755 "${stage}/bin/worker"
required_libs=(lib/librknnrt.so)
case "$plugin" in
  yolo26 | yolov8-pose) required_libs+=(lib/librga.so) ;;
esac
for lib in "${required_libs[@]}"; do
  if [[ ! -f "${stage}/${lib}" || -L "${stage}/${lib}" ]]; then
    echo "Error: ${lib} is missing from the worker output." >&2
    exit 1
  fi
done
if [[ "$plugin" == matcha-tts ]]; then
  chmod 0755 "${stage}/bin/espeak-ng"
  if [[ ! -f "${stage}/assets/text/espeak-ng-data/phontab" ]]; then
    echo "Error: Matcha eSpeak NG data is missing from the worker output." >&2
    exit 1
  fi
fi
mkdir -p "$output_dir"
mtime="$(git -C "$repo_root" log -1 --format=%ct 2>/dev/null || echo 0)"
archive_tmp="$(mktemp "${output_dir}/.rkserve-plugin-${plugin}.XXXXXXXX")"
tar --create --sort=name --owner=0 --group=0 --numeric-owner --mtime="@${mtime}" \
  --directory "$work_dir" "$plugin" | gzip -n -9 > "$archive_tmp"
chmod 0644 "$archive_tmp"
mv -f -- "$archive_tmp" "${output_dir}/rkserve-plugin-${plugin}.tar.gz"
archive_tmp=''
# Unique sidecar so matrix artifacts can merge; install.sh accepts it or SHA256SUMS.
(cd "$output_dir" && sha256sum "rkserve-plugin-${plugin}.tar.gz" > "rkserve-plugin-${plugin}.tar.gz.sha256")
echo "Packaged ${output_dir}/rkserve-plugin-${plugin}.tar.gz"
