# 第三方组件与模型来源

> 以下旧产物的哈希是历史来源证据，不是新版构建结果。新版包以随包生成的 BUILD-INFO.txt、BUILD-DEPS.txt 和 MODEL-SOURCES.txt 为准；转换复现及发布授权仍需验证。

[English](THIRD_PARTY_NOTICES.md) | 简体中文

本插件运行时不依赖外部项目目录。下列模型、运行库、文本资源和对应许可均随插件提供。

## Matcha 中英文模型与文本资源

- 来源：`https://modelscope.cn/models/dengcunqin/matcha_tts_zh_en_20251010`
- 上游模型包 SHA256：`271b804af570400d3bcdcb53bf6e53cc9f75180ee763b9f13eb5eaf2b0d086ef`
- 原始 `model-steps-3.onnx` SHA256：`524286bf6cf11be74329ae1c682ac69e34d6860c2ea9fd1290319d561540b16a`
- `lexicon.txt` SHA256：`599efdcdaff4df2a123ce988c2cb90abcd59b919ef609ce36eb587d68b7ca2c0`
- `tokens.txt` SHA256：`77fee8e5e5dd96b3547119e6159292c648e99e065c54c97777722e3ce710b9a4`
- 许可：Apache License 2.0，见 `Model-Apache-2.0`

`matcha-s64-fp16.rknn` 由原始 Matcha ONNX 固定为 80 个输入位置、最多 599 帧输出后，使用 RKNN-Toolkit2 为 RK3576 转换。转换后的 SHA256 为 `28d74f33fa8c8e35092af4f53518096d48f59a18111c2c7672dff8701a5befec`。

`matcha-duration.onnx` 是从同一模型抽取的 FP32 时长预测子图，用于校准 RKNN FP16 时长漂移。SHA256 为 `b5571a905d8fe75de28e685cc4c952dfca727b49c247a1d94f4bc3d911c537b6`。

Matcha-TTS 原始实现使用 MIT License，见 `Matcha-TTS-MIT`。

## Vocos

- 原始模型：`https://github.com/k2-fsa/sherpa-onnx/releases/download/vocoder-models/vocos-16khz-univ.onnx`
- 原始 ONNX SHA256：`b599142a1fb8ff03de3e84ac35ff537c619e56f4267a6fe894851a42844acf9e`
- RK3576 FP16 RKNN SHA256：`3aee1369a548ffde209bfc46ba36f06d60afac4c741e264ced3f61a7bdf73442`
- Vocos 原始实现使用 MIT License，见 `Vocos-MIT`

## eSpeak NG

- 来源提交：`https://github.com/csukuangfj/espeak-ng/commit/ed530aa113046142eb5115cf2fc9157854d0ffe1`
- 随附可执行文件 SHA256：`1beaa4b0bd8441761fc6db9849720d8ace802c450abf32ebb72517761a5b7e07`
- 对应完整源代码归档：`espeak-ng-source-ed530aa.zip`
- 源代码归档 SHA256：`e4e262cbe34f7fe21f91f1ba3397f2728e1f30eafbae7853f2b753a9ed13f0dd`
- 许可：GNU General Public License v3，见 `eSpeak-NG-GPL-3.0`

随插件保留的 `espeak-ng-data` 仅包含 `en-us` 运行所需的字典、音素表和语言配置，均由上述提交构建。

## ONNX Runtime

- 版本：1.26.0，Linux aarch64 CPU 构建
- 来源：`https://github.com/microsoft/onnxruntime/tree/v1.26.0`
- `libonnxruntime.so.1` SHA256：`8cac3d595f13c6c87c7d9b0590083f299bdd654def63960a58c8407b8b112328`
- 许可：MIT License，见 `ONNX-Runtime-MIT`

## RKNN Runtime

`librknnrt.so` 来自项目既有的 RK3576 RKNN Runtime 运行包，SHA256 为 `d31fc19c85b85f6091b2bd0f6af9d962d5264a4e410bfb536402ec92bac738e8`。其使用和再分发受 Rockchip 对应软件许可约束。

## 参考实现

文本前端行为和推理流程参考 sherpa-onnx（Apache License 2.0）及 `rkvoice-stream` 项目。插件代码已按 RKServe 协议以 Rust/C++ 重新实现，不在运行时导入上述项目。
