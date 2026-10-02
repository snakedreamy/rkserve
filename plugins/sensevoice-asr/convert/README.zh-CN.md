# SenseVoice-small 转换

[English](README.md) | 简体中文

## 来源与操作

固定 `lovemefan/SenseVoice-onnx` HF 提交 `8a5ee5b014950890a07246bc590a4f77b3ef67a4` 的 encoder、am.mvn、embedding.npy 与 SentencePiece BPE；完整 URL/SHA 见 `inputs.json`。原 encoder SHA `b1145eb7e9d2dbb9005fc99ed0313efebfadffea4aee1f4fa68b69a359126227`。

`vendor/sv_fix_shape.py` 将 speech 固定 `[1,344,560]`，删除 speech_lengths 输入并加入 INT64 `[344]` initializer，然后 shape inference。来自 rkvoice-stream 提交 `2266a05bdda637c13ab05f32ccdd6220621b0ec4`，原文件 SHA `c1f8cf7a6539d692fcf289a7d8ac605aa6f909c9e6f968fcf39af998a3c1a495`；副本只增加来源声明，Apache-2.0 许可随产物保留。

RKNN 参数：rk3576、opt3、float16、mean/std 默认0/1、无量化、无 dynamic_input，显式载入单个 speech `[1,344,560]`，输出 `[1,344,25055]`。tokens 按25055个 `id_to_piece(i)` 写成 `piece id\n`，am.mvn/embedding 原样复制。

输出 `sense-voice-encoder.rk3576.fp16.rknn` 和三个 text 文件。fixed ONNX（SHA `e81639fff8e672475bf550f1f681c0da10361cd7c5f3ddee1d34956788fb428d`）及 tokens 已在留存环境逐字节重现，脚本强制检查。

## 已证实与缺口

历史 RKNN 与工作区文件一致，实际转换命令/日志未保留，opt3 是参考转换脚本推断。lovemefan 是**第三方 ONNX 导出**，不是 Alibaba 原始 checkpoint→ONNX 的完整来源链；FunASR 模型许可与转换仓库 MIT 声明必须同时保留。不得将 SenseVoice-small 权重重许可为 MIT，发布前须审查 FunASR Model Open Source License Agreement v1.1 的义务。

## 调用与产物约定

在 GitHub 托管的 `ubuntu-24.04` x86_64 runner 用 `actions/setup-python` 选择 **Python 3.11**，再显式调用本目录 `convert.sh <绝对输出目录>`。输出目录必须为空或不存在；不支持覆盖旧产物。可用 `CONVERSION_PYTHON` 指定 3.11 可执行文件。本次实现未执行下载、转换、编译或依赖安装。

入口会创建临时工作区内的独立 venv（不进入输出），只安装 `plugins/tools/conversion-requirements.txt` 与本目录 `requirements.txt` 中逐项固定的 wheel；`--no-deps` 禁止自动引入未固定包，`pip check` 不通过即失败。Torch 使用官方 CPU wheel。RKNN x86 cp311 wheel SHA 固定为 `a05a8fd7515705ebdbb06a965c80b0090af61f7e716f1ed326a3aa5e313f23fb`。其他依赖固定发行版本，下载后保留 wheel 与实际 SHA；尚未对所有依赖 wheel 建立预先批准的哈希锁。

- `assets/`：严格匹配 `outputs.json` 的模型/数据文件，不包含运行库或 worker。
- `licenses/MODEL-SOURCES.txt`：来源、处理过程、许可与本说明的副本。
- `licenses/model-sources/inputs/`、`licenses/model-sources/INPUTS.json`：原始 ONNX/PT、数据、固定下载 URL、SHA256、许可。不会在成功后删除。
- `licenses/model-sources/conversion/`：本次实际转换脚本、图处理源码、许可证、输入输出清单及环境固定文件。
- `licenses/model-sources/environment/`：完整 wheel、固定 requirements、pip freeze、Python/系统信息与日志。
- `licenses/model-sources/intermediates/`：工作目录的 ONNX 图和混合量化中间文件。
- `licenses/model-sources/SHA256SUMS`：以上输入/源码/依赖的哈希（日志关闭后才生成哈希）。
- `licenses/OUTPUTS.sha256`：所有 assets 和 licenses 的哈希。失败保留 `licenses/CONVERSION-INCOMPLETE`；仅正常退出且该标记不存在时可交给后续验收。

**源码包必须保存整个 licenses/model-sources/，不能仅保存日志或模型。** 临时工作区在 `/tmp` 中，成功后自动清理，失败则显示其路径供诊断。输出根目录严格只有 `assets/` 与 `licenses/`；打包不能丢弃许可证目录中的原始输入/源码材料。工具不调用 `init_runtime`，不触碰 NPU，也不安装系统包。至少需要 git、Bash、Python3.11/venv 及宿主机可用的 OpenCV 系统动态库；转换的内存、磁盘峰值尚未在 GitHub runner 验证。

固定环境不等于已证实逐字节 RKNN 重建。历史音频/Pose 模型使用 arm64 编译器 `2.3.2 (@2025-04-03T08:26:16)`；新流程使用 x86_64 编译器 `2.3.2 (e045de294f@2025-04-07T19:48:25)`。不能把新产物的 SHA 改写成历史 SHA；必须另做质量/NPU验收。

## 授权与发布门禁

本流程从**已发布的 ONNX/PT** 开始，不声称提供原始训练数据、训练脚本或全部 preferred form for modification；成功转换也不是 GPL/AGPL 对应源码义务完成的法律证明。RKNN SDK 的生产使用/再分发授权仍须单独确认。不得因提供了许可证文本而宣称模型、图片、运行库自动获准再分发。
