# Changelog

English | [简体中文](CHANGELOG.zh-CN.md)

This project follows [Semantic Versioning](https://semver.org/) and [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0] — release preparation

Initial open-source version; this entry does not mean assets or images have been published.

- `rkserve-core`: NPU scheduling, plugin lifecycle, asynchronous jobs, structured events, API key authentication and a unified HTTP boundary.
- React management console, English by default, with a persistent English/Simplified Chinese switch.
- Optional plugin display translations, with English API values and diagnostic messages.
- Repository reorganized into `server/` and `plugins/`; contributor guidelines and submission templates removed.
- RK3576 plugins: `yolo26`, `yolov8-pose`, `sensevoice-asr`, `zipformer-asr` and `matcha-tts`.
- Per-plugin binary packages and installer replace the old asset-only archives. Actions has packaged all five plugins; no plugin GitHub Release has been published.
- Server-only container image; plugin packages retain their own licenses and source obligations.
- React Router, transitive frontend build dependencies and Rust `h2` updated to address known dependency advisories.
- Multi-stage `Containerfile`, rootless Podman/Quadlet examples and an ARM64 GHCR publishing workflow.

[0.1.0]: https://github.com/snakedreamy/rkserve/releases/tag/v0.1.0
