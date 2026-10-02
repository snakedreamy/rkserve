# YOLOv8 Pose RKNN 插件

[English](README.md) | 简体中文

本插件在 RK3576 上运行 YOLOv8n-Pose，检测人物并返回边界框与 COCO 17 点人体骨架。

## 推理边界

- JPEG/PNG 解码：CPU
- Letterbox 缩放：Rockchip RGA，失败时回退到 CPU
- 主干、检测头和姿态头：RK3576 NPU
- 后处理：CPU

## 构建

```bash
./plugins/yolov8-pose/build.sh /absolute/path/to/worker-output
```

RKNN/RGA 头文件和运行库由 Rockchip 提供。模型遵循 `licenses/` 中的 Ultralytics 许可。
