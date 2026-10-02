#!/usr/bin/env bash
# Build the YOLO26 worker and stage bin/ and lib/ into the given directory.
set -euo pipefail

crate=rkserve-yolo26-worker
deps=(rknpu2 librga)
libs=(rknpu2/lib/librknnrt.so librga/lib/librga.so)
# shellcheck source-path=SCRIPTDIR source=../tools/build-worker.sh
source "$(dirname "$0")/../tools/build-worker.sh"
