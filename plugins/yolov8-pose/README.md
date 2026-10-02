# YOLOv8 Pose RKNN plugin

English | [简体中文](README.zh-CN.md)

This plugin runs YOLOv8n-Pose on RK3576. It detects people and returns bounding boxes with COCO 17-keypoint skeletons.

## Inference boundary

- JPEG/PNG decoding: CPU
- Letterbox resize: Rockchip RGA, with a CPU fallback
- Backbone, detection head and pose head: RK3576 NPU
- Post-processing: CPU

## Build

```bash
./plugins/yolov8-pose/build.sh /absolute/path/to/worker-output
```

RKNN/RGA headers and runtime libraries are provided by Rockchip. The model is subject to the Ultralytics license included under `licenses/`.
