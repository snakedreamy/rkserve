#!/usr/bin/env bash
# Build the SenseVoice worker and stage bin/ and lib/ into the given directory.
set -euo pipefail

crate=rkserve-sensevoice-asr-worker
deps=(rknpu2)
libs=(rknpu2/lib/librknnrt.so)
# shellcheck source-path=SCRIPTDIR source=../tools/build-worker.sh
source "$(dirname "$0")/../tools/build-worker.sh"
