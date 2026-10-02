# 更新日志

[English](CHANGELOG.md) | 简体中文

本项目遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)，格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## [0.1.0] — 2026-10-03

首个公开版本。

- `rkserve-core`：NPU 核心调度、插件生命周期、异步 Job、结构化事件、API Key 鉴权与统一 HTTP 边界。
- React 管理控制台，默认英文，可持久保存 English／简体中文偏好。
- 插件显示文本支持可选翻译，API 标识符和诊断信息保持英文。
- 仓库整理为 `server/` 本体和 `plugins/` 插件源码。
- 插件：`yolo26`、`yolov8-pose`、`sensevoice-asr`、`zipformer-asr`、`matcha-tts`（均针对 RK3576）。
- 用 `plugins/install.sh` 从 GitHub Release 安装插件包。
- `linux/arm64` 容器镜像只包含 Core 和控制台；插件安装在宿主机上，再挂载进容器。
- 多阶段 `Containerfile`、Rootless Podman／Quadlet 部署示例，以及发布 arm64 镜像到 GHCR 的工作流。

[0.1.0]: https://github.com/snakedreamy/rkserve/releases/tag/v0.1.0
