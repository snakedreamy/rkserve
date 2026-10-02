# 更新日志

[English](CHANGELOG.md) | 简体中文

本项目遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)，格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## [0.1.0] — 发布准备

首个开源版本；该条目不表示资源和镜像已经发布。

- `rkserve-core`：NPU 核心调度、插件生命周期、异步 Job、结构化事件、API Key 鉴权与统一 HTTP 边界；
- React 管理控制台，默认英文，可持久保存 English／简体中文偏好；
- 插件显示文本支持可选翻译，API 基础值和诊断保持英文；
- 仓库整理为 `server/` 和 `plugins/` 两个主目录，移除贡献规范和提交模板；
- 插件：`yolo26`、`yolov8-pose`、`sensevoice-asr`、`zipformer-asr`、`matcha-tts`（均针对 RK3576）；
- 以可直接安装的独立插件包替代旧资源归档。Actions 已打出五个插件包，尚未发布插件 GitHub Release；
- 本体镜像不再包含插件，各插件包保留自己的许可证与对应源码义务；
- 更新 React Router、前端构建链间接依赖及 Rust `h2`，处理已知依赖安全公告；
- 多阶段 `Containerfile`、Rootless Podman／Quadlet 部署示例，以及发布 arm64 镜像到 GHCR 的工作流。

[0.1.0]: https://github.com/snakedreamy/rkserve/releases/tag/v0.1.0
