# Matcha 中英文 TTS 转换

[English](README.md) | 简体中文

## 来源与操作

模型作者为 dengcunqin，发布页 `https://modelscope.cn/models/dengcunqin/matcha_tts_zh_en_20251010`。使用 sherpa-onnx `tts-models` 的 `matcha-icefall-zh-en.tar.bz2`（SHA `271b804af570400d3bcdcb53bf6e53cc9f75180ee763b9f13eb5eaf2b0d086ef`）；Vocos 来自 `vocoder-models/vocos-16khz-univ.onnx`（SHA `b599142a1fb8ff03de3e84ac35ff537c619e56f4267a6fe894851a42844acf9e`）。完整 URL/哈希见 `inputs.json`。

1. 解包原始 `model-steps-3.onnx`，保留 INT64 scalar `/ReduceMax_output_0` 为第二输出。仅在此图上提取 `x,x_length,length_scale`→长度的 FP32 duration 图。
2. `vendor/fix_matcha_rknn.py --seq-len 80 --x-len 64`：探测原图、onnxsim 固定输入、Range 常量化、Ceil→Neg/Floor/Neg、Slice end 固定599、`default_rng(42)` 生成固定 `[1,80,600]` 噪声、shape inference。副本来自 rkvoice-stream 提交 `2266a05bdda637c13ab05f32ccdd6220621b0ec4`，原文件 SHA `c714fc11f073a12e7102a0f1edf203c2e41ddc14e4c688cb44e27ee3b4cb76be`，除中文来源头外未改；完整 Apache-2.0 许可随输出附带。
3. 声学 RKNN：rk3576、opt0、float16、无量化、mean/std 默认0/1、无 dynamic_input；输出 mel `[1,80,599]` 与实际长度。文件名虽然是 `matcha-s64-fp16.rknn`，图必须是带长度的版本。
4. Vocos：rk3576、opt3、float16、single_core_mode=False、无量化；mels 固定 `[1,80,600]`，输出三路 `[1,513,600]`；不应用仅针对 RK3588 的 Clip 补丁。
5. lexicon/tokens 从模型归档复制。八个 en-us eSpeak 数据文件由 Worker 构建与可执行文件一起生成；转换步骤不再用旧 phondata 覆盖新构建。 同时校验并保留 csukuangfj/espeak-ng 提交 `ed530aa113046142eb5115cf2fc9157854d0ffe1` 的完整源 zip（SHA `e4e262cbe34f7fe21f91f1ba3397f2728e1f30eafbae7853f2b753a9ed13f0dd`），供后续 GPL 源码包。

输出 `matcha-duration.onnx`、`matcha-s64-fp16.rknn`、`vocos-16khz-600-fp16.rknn` 及 text 数据。eSpeak 的 aarch64 executable、匹配数据和运行库需另由插件构建流程提供；本入口不会生成 `bin/espeak-ng`。

## 已证实与缺口

从加过长度输出的图中抽出的 duration 图会做哈希校验。随后的 probe-first 改图由 `fix_matcha_rknn.py` 内的 ONNX Runtime 验证（输出形状与余弦相似度）；改图结果作为中间产物保留，不当作固定输入哈希。实际历史 RKNN 转换命令/日志缺失：opt0/opt3/single_core 参数是参考脚本推断，尚不能证明完全重现历史 RKNN。eSpeak 已知本地构建数据与归档八个文件一致，但完整构建工具链、原始模型训练工程和 Vocos 权重训练来源尚不完整。

模型/文本 Apache-2.0、Matcha/Vocos 实现 MIT、eSpeak executable/data GPL-3.0，各许可不相互替代。

## 调用与产物约定

在 GitHub 托管的 `ubuntu-24.04` x86_64 runner 用 `actions/setup-python` 选择 **Python 3.11**，再显式调用本目录 `convert.sh <绝对输出目录>`。输出目录必须为空或不存在；不支持覆盖旧产物。可用 `CONVERSION_PYTHON` 指定 3.11 可执行文件。

入口会创建临时工作区内的独立 venv（不进入输出），只安装 `plugins/tools/conversion-requirements.txt` 与本目录 `requirements.txt` 中逐项固定的 wheel；`--no-deps` 禁止自动引入未固定包，`pip check` 不通过即失败。Torch 使用官方 CPU wheel。RKNN x86 cp311 wheel SHA 固定为 `a05a8fd7515705ebdbb06a965c80b0090af61f7e716f1ed326a3aa5e313f23fb`。其他依赖固定发行版本，下载后保留 wheel 与实际 SHA。

- `assets/`：严格匹配 `outputs.json` 的模型/数据文件，不包含运行库或 worker。
- `licenses/MODEL-SOURCES.txt`：来源、处理过程、许可与本说明的副本。
- `licenses/model-sources/inputs/`、`licenses/model-sources/INPUTS.json`：原始 ONNX/PT、数据、固定下载 URL、SHA256、许可。不会在成功后删除。
- `licenses/model-sources/conversion/`：本次实际转换脚本、图处理源码、许可证、输入输出清单及环境固定文件。
- `licenses/model-sources/environment/`：完整 wheel、固定 requirements、pip freeze、Python/系统信息与日志。
- `licenses/model-sources/intermediates/`：工作目录的 ONNX 图和混合量化中间文件。
- `licenses/model-sources/SHA256SUMS`：以上输入/源码/依赖的哈希（日志关闭后才生成哈希）。
- `licenses/OUTPUTS.sha256`：所有 assets 和 licenses 的哈希。失败保留 `licenses/CONVERSION-INCOMPLETE`；仅正常退出且该标记不存在时可交给后续验收。

**源码包必须保存整个 licenses/model-sources/，不能仅保存日志或模型。** 临时工作区在 `/tmp` 中，成功后自动清理，失败则显示其路径供诊断。输出根目录严格只有 `assets/` 与 `licenses/`；打包不能丢弃许可证目录中的原始输入/源码材料。工具不调用 `init_runtime`，不触碰 NPU，也不安装系统包。至少需要 git、Bash、Python3.11/venv 及宿主机可用的 OpenCV 系统动态库。

固定环境不等于已证实逐字节 RKNN 重建。历史音频/Pose 模型使用 arm64 编译器 `2.3.2 (@2025-04-03T08:26:16)`；新流程使用 x86_64 编译器 `2.3.2 (e045de294f@2025-04-07T19:48:25)`。不能把新产物的 SHA 改写成历史 SHA；必须另做质量/NPU验收。
