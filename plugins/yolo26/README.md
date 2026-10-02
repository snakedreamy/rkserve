# YOLO26 RKNN plugin

English | [简体中文](README.zh-CN.md)

This plugin runs the official Ultralytics YOLO26 RKNN exports on RK3576. It is
self-contained and does not import code from a local Ultralytics checkout at
build or runtime.

## Inference boundary

- JPEG/PNG decoding: CPU (`image` crate)
- 640 x 640 letterbox resize: Rockchip RGA, with an exact CPU fallback
- Backbone, detection head, box decoding and class sigmoid: RK3576 NPU
- Confidence filtering and class-aware NMS: CPU

The supplied RKNN models have a single FP16 output and were exported with
`end2end: false`. Loading derives the output channel count from each model's
class metadata and rejects incompatible input or output tensors instead of
silently producing invalid detections.

## Models

RKServe recursively discovers `.rknn` files under `assets/models/`; the model
directory must also contain the Ultralytics `metadata.yaml` produced by export.
The relative model path is the stable configuration value, so models with the
same input size but different names or detection classes can coexist.

The worker currently accepts 640 x 640, non-end-to-end detection exports. It
reads the ordered class names from the selected model's metadata and validates
that they match the RKNN output. Only the selected model is loaded into memory.
When the plugin is running, RKServe drains the current worker and reloads the
new model on the same NPU mask.

To add a model, copy its export directory (the `.rknn` file and sibling
`metadata.yaml`) below `assets/models/`, then restart Core so the registry can
discover it. Do not edit `plugin.toml` or the frontend for each new model.

## Build

```bash
./plugins/yolo26/build.sh /absolute/path/to/worker-output
```

The native inference and post-processing implementation lives in
`worker/native/bridge.cc`. Bundled RKNN/RGA headers and runtime libraries are
provided by Rockchip. Ultralytics model metadata and exported model artifacts
are subject to the Ultralytics license included under `licenses/`.
