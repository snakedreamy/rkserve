# 第三方说明

> 以下旧产物的哈希是历史来源证据，不是新版构建结果。新版包以随包生成的 BUILD-INFO.txt、BUILD-DEPS.txt 和 MODEL-SOURCES.txt 为准；转换复现及发布授权仍需验证。

[English](THIRD_PARTY_NOTICES.md) | 简体中文

- Rockchip `rknn_model_zoo` YOLOv8-Pose 示例前后处理源码（`vendor/yolov8_pose` 和 `vendor/utils`）：Apache-2.0，见 `rknn-model-zoo-LICENSE`。此许可不改变模型或 RKNN SDK 的许可。
- `yolov8_pose.rknn` 派生自 Ultralytics YOLOv8-Pose 权重：AGPL-3.0，见 `ultralytics-AGPL-3.0.txt`。
- `libturbojpeg.a` 和 `turbojpeg.h`：libjpeg-turbo，BSD 风格许可证，见 `libjpeg-turbo-LICENSE.md`。
- `stb_image.h` / `stb_image_write.h`：MIT 或公有领域，见 `stb_image-LICENSE.txt`。
- `librknnrt.so` 及 RKNN 头文件：Rockchip RKNN SDK License，见 `RKNN-SDK-LICENSE`；未确认的再分发范围见插件总览（plugins/README.zh-CN.md）。
- `librga.so` 及 RGA 头文件：Apache-2.0，见 `RGA-Apache-2.0`。

留存的转换工作区对应 model-zoo 提交 [`bad6c733`](https://github.com/airockchip/rknn_model_zoo/tree/bad6c7334531becaf90a561988519b7bec34d0ab/examples/yolov8_pose)。
未修改的 `python/convert.py` SHA-256 为 `b3c8fe3855339e9b7c497717c2de84e6d65e4017e6739f4df24080878bc45a8e`。
留存的 `yolov8n-pose.onnx` 与上游 README 下载文件一致，SHA-256 为 `308495ebe4416b40adf376485252a7b8ba7933a169368b31e74e0f977ded8663`；
留存的输出与随附 RKNN 一致，SHA-256 为 `1ae418119445651ebea28a1dfce9a3104c0135def338e48038b5ea1f9b42fbd8`。
原执行命令和转换日志未留存，因此这些是来源证据，不表示已完成转换复现。
