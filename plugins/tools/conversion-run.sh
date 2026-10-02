#!/usr/bin/env bash
# 转换专用入口；不依赖或改动现有插件构建工具。
set -euo pipefail
if [[ $# != 2 || $2 != /* ]]; then
  echo 'Usage: conversion-run.sh <recipe-directory> <absolute-output-directory>' >&2
  exit 2
fi
recipe=$(realpath "$1")
tools=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
plugin=$(basename "$(dirname "$recipe")")
# 防止把 recipe 复制进其自己的输出子目录造成递归复制。
candidate=$(realpath -m "$2")
if [[ "$candidate" == "$recipe" || "$candidate" == "$recipe/"* ]]; then
  echo 'Output directory must not be inside the conversion recipe directory' >&2
  exit 2
fi
python=${CONVERSION_PYTHON:-python3.11}
"$python" -c 'import platform,sys; assert sys.version_info[:2] == (3,11), "Python 3.11 is required"; assert platform.system() == "Linux" and platform.machine() == "x86_64", "Conversion requires Linux x86_64"'
# 不允许复用旧产物，防止失败时把残留文件误判为本次成功结果。
if [[ -L $2 || ( -e $candidate && ( ! -d $candidate || -n $(find "$candidate" -mindepth 1 -maxdepth 1 -print -quit) ) ) ]]; then
  echo 'Output directory must be absent or empty' >&2
  exit 2
fi
mkdir -p -- "$2"
out=$(realpath "$2")
sources="$out/licenses/model-sources"
mkdir -p "$out/assets" "$sources"/{conversion/plugins,inputs,environment/wheels}
if [[ -n "${CONVERSION_INPUT_CACHE:-}" && -d "$CONVERSION_INPUT_CACHE" ]]; then
  cp -a "$CONVERSION_INPUT_CACHE"/. "$sources/inputs/"
fi
printf 'Conversion incomplete; do not publish.\n' > "$out/licenses/CONVERSION-INCOMPLETE"
# 临时环境在输出目录外，避免产物包混入 venv、缓存或 Git 工作区。
work=$(mktemp -d /tmp/rkserve-conversion.XXXXXXXX)
cleanup() {
  status=$?
  if [[ -f "$work/conversion.log" ]]; then
    cp "$work/conversion.log" "$sources/environment/conversion.log"
  fi
  if [[ $status == 0 ]]; then
    rm -rf -- "$work"
  else
    echo "Conversion failed; marker retained. Diagnostic workspace: $work" >&2
  fi
}
trap cleanup EXIT
mkdir -p "$sources/conversion/plugins/$plugin" "$sources/conversion/plugins/tools"
cp -a "$recipe" "$sources/conversion/plugins/$plugin/convert"
# 仅保留本次使用的转换工具，不复制正在开发的其他工具。
cp "$tools"/conversion-{run.sh,requirements.txt} "$tools/conversion_common.py" "$sources/conversion/plugins/tools/"
recipe="$sources/conversion/plugins/$plugin/convert"
tools="$sources/conversion/plugins/tools"
cp "$recipe"/licenses/* "$out/licenses/"
cat "$tools/conversion-requirements.txt" "$recipe/requirements.txt" > "$sources/environment/requirements.txt"
export PYTHONDONTWRITEBYTECODE=1 PYTHONHASHSEED=0
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-0} CONVERSION_WORK_DIR="$work"
export TMPDIR="$work/tmp" XDG_CACHE_HOME="$work/cache" TORCH_HOME="$work/torch"
export YOLO_CONFIG_DIR="$work/ultralytics" YOLO_AUTOINSTALL=false
export PIP_CONFIG_FILE=/dev/null PIP_DISABLE_PIP_VERSION_CHECK=1
mkdir -p "$TMPDIR" "$XDG_CACHE_HOME" "$TORCH_HOME" "$YOLO_CONFIG_DIR"
"$python" -m venv "$work/venv"
py="$work/venv/bin/python"
{
  # --no-deps 防止解析器引入未固定依赖；安装后 pip check 必须通过。
  "$py" -m pip download --no-deps --only-binary=:all: \
    --index-url https://pypi.org/simple --extra-index-url https://download.pytorch.org/whl/cpu \
    --dest "$sources/environment/wheels" -r "$sources/environment/requirements.txt"
  wheel="$sources/environment/wheels/rknn_toolkit2-2.3.2-cp311-cp311-manylinux_2_17_x86_64.manylinux2014_x86_64.whl"
  printf '%s  %s\n' a05a8fd7515705ebdbb06a965c80b0090af61f7e716f1ed326a3aa5e313f23fb "$wheel" | sha256sum -c -
  "$py" -m pip install --no-index --no-deps --find-links "$sources/environment/wheels" \
    -r "$sources/environment/requirements.txt"
  "$py" -m pip check
  "$py" -m pip freeze --all > "$sources/environment/pip-freeze.txt"
  "$py" -VV > "$sources/environment/python.txt"
  uname -a > "$sources/environment/uname.txt"
  cp /etc/os-release "$sources/environment/os-release"
  printf 'SOURCE_DATE_EPOCH=%s\nPYTHONHASHSEED=0\nYOLO_AUTOINSTALL=false\n' "$SOURCE_DATE_EPOCH" > "$sources/environment/variables.txt"
  cd "$work"
  "$py" "$recipe/convert.py" "$out"
} 2>&1 | tee "$work/conversion.log"
# 等待管道关闭后复制日志，避免哈希清单覆盖一个仍在写入的文件。
cp "$work/conversion.log" "$sources/environment/conversion.log"
"$py" "$tools/conversion_common.py" finalize "$recipe" "$out"
rm "$out/licenses/CONVERSION-INCOMPLETE"
echo "Conversion complete: $out; output contains only assets/ and licenses/, including retained source materials."
