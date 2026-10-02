# YOLOv8-Pose 混合量化转换

[English](README.md) | 简体中文

## 来源与操作

使用 model-zoo 官方 Filez 的 `yolov8n-pose.onnx`，SHA `308495ebe4416b40adf376485252a7b8ba7933a169368b31e74e0f977ded8663`。它是 opset11、四输出的特殊提取图，**不是原版 Ultralytics 默认 ONNX**。

参考 model-zoo `bad6c7334531becaf90a561988519b7bec34d0ab` 的 convert.py：等价于 `convert.py <onnx> rk3576 i8 <output>`。固定其 COCO subset20 清单顺序和全部20张图片 SHA；保留原始图片、清单与上游脚本，不只保留临时绝对路径列表。

rk3576，mean0/std255，opt3，float16；调用 hybrid_quantization_step1(dataset, proposal=False, custom_hybrid=...)，再 step2。三段 `/model.22/cv4.{0,1,2}/cv4.{0,1,2}.0/act/Mul_output_0` 至 `/model.22/Concat_6_output_0` 保留浮点。不是普通 INT8 build，也不是 fp 模式。每个 SDK 返回码和 step1 的 .model/.data/.quantization.cfg 文件都验证。

输出 `yolov8_pose.rknn` 与 `yolov8_pose_labels_list.txt`（严格 `person\n`）。RKNN 三路检测输出应 INT8，关键点输出未量化；检查四路形状并保留混合量化中间文件。

## 已证实与缺口

历史 RKNN 的 INT8/FP16 混合类型已从文件证实，模型/参考脚本与留存文件一致。但**原命令和实际校准执行证据缺失**，subset20 是未改参考脚本规定的数据，不是从历史日志证明。文件复制时间不能当构建时间。

原 ONNX 标记 `onnx.utils.extract_model`；官方 PT 经 `airockchip/ultralytics_yolov8` 到此 ONNX 的确切提交/命令仍缺。此脚本不补造该来源链。模型 AGPL-3.0，model-zoo 代码 Apache-2.0，COCO 原图版权归原作者；图片是否可随源码/产物再分发须单独审查，不能用仓库 Apache 许可替代。

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
