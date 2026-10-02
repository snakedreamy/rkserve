# Zipformer 中英文流式 ASR 转换

[English](README.md) | 简体中文

## 来源与操作

使用 Rockchip model-zoo 发布的三个 `*-epoch-99-avg-1.onnx`，URL 为 Filez 固定 delivery 路径（完整 SHA/URL 见 `inputs.json`）。保留 model-zoo 提交 `bad6c7334531becaf90a561988519b7bec34d0ab` 的原 convert.py 与 vocab，模型和参考代码标注 Apache-2.0。

逐个静态 ONNX 执行 RKNN config→load_onnx→build→export，rk3576、opt3、float16、无量化、mean/std 默认0/1，无 dynamic_input。相当于参考 `convert.py <onnx> rk3576 fp <output>`，但本脚本验证所有返回码。

encoder 有36个输入/36个输出，首输入 `[1,103,80]`；decoder 输入 `[1,2]` 输出 `[1,512]`；joiner 两路 `[1,512]` 输出 `[1,6254]`。输出 `encoder.rknn`、`decoder.rknn`、`joiner.rknn` 与原样 vocab.txt。

## 已证实与缺口

这三个文件与 k2 HF `csukuangfj/k2fsa-zipformer-bilingual-zh-en-t` 的同名 ONNX **哈希/大小不同**。没有找到 k2 原图→Rockchip 静态/优化图的确切处理代码；本脚本直接使用 Rockchip 版本，绝不能宣称已补齐这一链路。历史 RKNN 与本地工作区哈希一致，但原始命令/日志未保留；opt3 是固定 toolkit 默认值，而不是从日志取证。

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
