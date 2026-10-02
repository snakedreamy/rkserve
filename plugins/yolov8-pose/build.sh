#!/usr/bin/env bash
# Build the YOLOv8-Pose worker and stage bin/ and lib/ into the given directory.
set -euo pipefail

crate=rkserve-yolov8-pose-worker
deps=(rknpu2 librga jpeg_turbo)
libs=(rknpu2/lib/librknnrt.so librga/lib/librga.so)
# shellcheck source-path=SCRIPTDIR source=../tools/build-worker.sh
source "$(dirname "$0")/../tools/build-worker.sh"
