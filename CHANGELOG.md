# Changelog

English | [简体中文](CHANGELOG.zh-CN.md)

This project follows [Semantic Versioning](https://semver.org/) and [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0] — 2026-10-03

First public release.

- `rkserve-core`: NPU scheduling, plugin lifecycle, asynchronous jobs, structured events, API key authentication and a unified HTTP boundary.
- React management console, English by default, with a persistent English/Simplified Chinese switch.
- Optional plugin display translations; API identifiers and diagnostic messages stay in English.
- Repository layout: `server/` application and `plugins/` plugin sources.
- RK3576 plugins: `yolo26`, `yolov8-pose`, `sensevoice-asr`, `zipformer-asr` and `matcha-tts`.
- Plugin packages installed with `plugins/install.sh` from GitHub Releases.
- `linux/arm64` container image with Core and the console; plugins are installed on the host and mounted into the container.
- Multi-stage `Containerfile`, rootless Podman/Quadlet examples and an ARM64 GHCR publishing workflow.

[0.1.0]: https://github.com/snakedreamy/rkserve/releases/tag/v0.1.0
