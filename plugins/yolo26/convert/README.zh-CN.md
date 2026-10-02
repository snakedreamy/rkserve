# 五种 YOLO26 FP16 模型转换

[English](README.md) | 简体中文

## 来源与操作

固定官方 `ultralytics/assets` 发布 `v8.4.0` 的 yolo26n/s/m/l/x.pt，使用 GitHub release digest（见 `inputs.json`）。n 已重新下载验证；其余四个 digest 来自 GitHub API，尚未本地重新下载。

固定 Ultralytics 8.4.103，额外保留 v8.4.103 提交 `bdc65f629a533f494da2a73cb8e201ea11943b54` 的完整 git archive，预先核验 SHA `dd8fd8b61cae9d046aad3f34f1d0ac48ec8d2b249ee20150c0764f401ec58abf`。本转换脚本基于 AGPL-3.0 的 Exporter 流程，沿用同许可；完整源码、PT、转换脚本和许可都要进入后续源码包。

等价模型参数：format=rknn、name=rk3576、imgsz640、batch1、quantize16、end2end=False、dynamic=False、nms=False、device=cpu、opset19。保留 `format=rknn` 上下文，不改成另一路 ONNX half export。覆盖 export_rknn 阶段以显式调用 onnxslim；上游的 simplifier 异常会被警告后忽略，本流程不允许这样悄悄继续。

RKNN mean `[0,0,0]` / std `[255,255,255]`，opt3、float16、无量化、rknn_batch_size=1。输入 `[1,3,640,640]`，输出 `[1,84,8400]`。写入 `yolo26{n,s,m,l,x}_rknn_fp16_640`，内含 `yolo26{size}-rk3576.rknn` 与 metadata.yaml；不是上游默认 `_rknn_model` 目录。

metadata description 仅包含 COCO 公共引用；date 使用 `SOURCE_DATE_EPOCH`（默认0），同样应用到 ONNX metadata，不保留作者本地训练路径。worker 递归发现带 sibling metadata 的模型，此目录布局符合其契约。

## 已证实与缺口

历史 metadata 为8.4.103，原 RKNN compiler 也是 x86 2.3.2；没有原转换日志或 ONNX，不能证明该脚本重现历史二进制。日期规范化本来就会改变 metadata SHA。原 exporter 仅要求 toolkit>=2.3.2、ONNX<1.19；本流程更严格锁版本。

**PT 是发布的训练权重，不等于训练源码齐全。** 保留官方训练实现不代表已获得训练数据、准确训练配方或所有 preferred source；AGPL对应源码范围仍须法律审核。源码 archive 校验若因 Git 行为差异失败，应调查而非删除哈希门禁。

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
