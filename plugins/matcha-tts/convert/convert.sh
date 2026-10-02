#!/usr/bin/env bash
# 仅接收绝对输出路径；依赖安装与转换仅在显式调用时执行。
set -euo pipefail
if [[ $# != 1 || $1 != /* ]]; then
  echo 'Usage: convert.sh <absolute-output-directory>' >&2
  exit 2
fi
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
exec bash "$here/../../tools/conversion-run.sh" "$here" "$1"
