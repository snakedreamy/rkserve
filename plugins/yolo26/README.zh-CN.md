# YOLO26 RKNN 插件

[English](README.md) | 简体中文

本插件在 RK3576 上运行 Ultralytics 官方 YOLO26 RKNN 导出模型。插件自包含，构建和运行均不导入本地 Ultralytics 项目。

## 推理边界

- JPEG/PNG 解码：CPU（`image` crate）。
- 640 × 640 letterbox 缩放：Rockchip RGA，提供对应 CPU 回退。
- 主干、检测头、框解码和类别 sigmoid：RK3576 NPU。
- 置信度过滤与分类别 NMS：CPU。

随附 RKNN 模型只有一个 FP16 输出，导出时使用 `end2end: false`。加载时根据类别元数据推导输出通道数，并拒绝不兼容的输入/输出张量，不会静默产生无效检测结果。

## 模型

RKServe 递归发现 `assets/models/` 下的 `.rknn` 文件；同目录必须有导出时生成的 Ultralytics `metadata.yaml`。相对模型路径是稳定配置值，因而同尺寸、不同名称或类别的模型可以共存。

Worker 当前接受 640 × 640、非端到端检测导出。它读取所选模型元数据中的有序类别名，验证与 RKNN 输出匹配，仅将选定模型载入内存。运行时更换模型会排空当前 Worker，并在同一 NPU 核心掩码上重新加载。

新增模型时，将 `.rknn` 和同目录的 `metadata.yaml` 一起复制到 `assets/models/` 下，再重启 Core 刷新注册表；无需逐个修改 `plugin.toml` 或前端。

## 构建

```bash
./plugins/yolo26/build.sh /absolute/path/to/worker-output
```

原生推理和后处理位于 `worker/native/bridge.cc`。RKNN/RGA 头文件和运行库由 Rockchip 提供。Ultralytics 模型元数据和导出模型适用 `licenses/` 中的 Ultralytics 许可证。
