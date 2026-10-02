#!/usr/bin/env bash
# Absolute output path only; dependency install and conversion run when this script is invoked.
set -euo pipefail
if [[ $# != 1 || $1 != /* ]]; then
  echo 'Usage: convert.sh <absolute-output-directory>' >&2
  exit 2
fi
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
exec bash "$here/../../tools/conversion-run.sh" "$here" "$1"
