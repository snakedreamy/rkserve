# Third-party notices

> The old artifact hashes below are historical provenance evidence, not results of the new build. New packages use their generated BUILD-INFO.txt, BUILD-DEPS.txt and MODEL-SOURCES.txt. Conversion reproducibility and redistribution authorization still require verification.

English | [简体中文](THIRD_PARTY_NOTICES.zh-CN.md)

- Rockchip `rknn_model_zoo` YOLOv8-Pose demo pre/post-processing sources under
  `vendor/yolov8_pose` and `vendor/utils`: Apache-2.0. See
  `rknn-model-zoo-LICENSE`. This does not relicense the model or RKNN SDK.
- `yolov8_pose.rknn` is derived from Ultralytics YOLOv8-Pose weights: AGPL-3.0.
  See `ultralytics-AGPL-3.0.txt`.
- `libturbojpeg.a` and `turbojpeg.h`: libjpeg-turbo, BSD-style licenses. See
  `libjpeg-turbo-LICENSE.md`.
- `stb_image.h` / `stb_image_write.h`: MIT or public domain. See `stb_image-LICENSE.txt`.
- `librknnrt.so` and RKNN headers: Rockchip RKNN SDK License; see `RKNN-SDK-LICENSE`
  and the plugin overview (plugins/README.md) for the unresolved redistribution scope.
- `librga.so` and RGA headers: Apache-2.0; see `RGA-Apache-2.0`.

The retained conversion workspace matches model-zoo commit
[`bad6c733`](https://github.com/airockchip/rknn_model_zoo/tree/bad6c7334531becaf90a561988519b7bec34d0ab/examples/yolov8_pose).
Its unmodified `python/convert.py` has SHA-256 `b3c8fe3855339e9b7c497717c2de84e6d65e4017e6739f4df24080878bc45a8e`.
The retained `yolov8n-pose.onnx` matches the upstream README download, SHA-256
`308495ebe4416b40adf376485252a7b8ba7933a169368b31e74e0f977ded8663`;
the retained output matches the shipped RKNN, SHA-256
`1ae418119445651ebea28a1dfce9a3104c0135def338e48038b5ea1f9b42fbd8`.
The original command and conversion log were not retained, so this is provenance evidence, not a claim of a reproduced conversion.
