# RKServe plugins

English | [简体中文](README.zh-CN.md)

Plugin sources, third-party licenses, model conversion and packaging tools live here, separate from the `server/` application.

## Installation

Each package contains `plugin.toml`, `bin/worker`, `lib/`, `assets/` and `licenses/`. Users do not need to compile anything. Installation requires Bash, Python 3, curl, tar, sha256sum and flock. Running a plugin requires a compatible ARM64 system, an RK3576 and the host NPU driver.

```bash
plugins/install.sh --dest "$HOME/rkserve/plugins" yolo26 sensevoice-asr
```

The installer downloads `rkserve-plugin-*.tar.gz` from GitHub Releases (default tag `plugins-v0.1.0`) and verifies SHA-256. Use `--from DIR` to install from local archives instead; the directory must contain the archives and either `SHA256SUMS` or each archive's `*.tar.gz.sha256` sidecar. Stop RKServe before updating plugins and restart it afterward. Plugins are native programs, not a security sandbox: install packages only from trusted sources.

## Sources and builds

- `<plugin>/worker/`: Rust worker and C/C++ bridge.
- `<plugin>/convert/`: model conversion steps and source records.
- `<plugin>/licenses/`: component licenses and source notices.
- `vendor/`: third-party sources shared by multiple plugins.
- `deps.lock`: pinned download URLs and SHA-256 hashes for third-party headers and runtime libraries.
- `tools/`: download, build and packaging scripts.

`server/` and `plugins/` use separate Cargo workspaces. Plugins reference `server/protocol` in this repository. Core does not link plugins, RKNN or models.

Plugin workers still need to be compiled by the publisher; users install the resulting binaries. ARM64 compilation and x86_64 model conversion run in GitHub Actions. The manual worker build entry point is:

```bash
plugins/yolo26/build.sh /absolute/path/to/worker-output
```

Downloaded dependencies are stored in the Git-ignored `.deps/` directory. Do not commit it or conversion work directories.

## Licenses

Each plugin keeps its own texts under `<plugin>/licenses/`. In short:

| Component | License |
| --- | --- |
| Most first-party worker sources | Apache-2.0 |
| Matcha worker and eSpeak NG | GPL-3.0 |
| YOLO26 and YOLOv8-Pose models | AGPL-3.0 |
| SenseVoice model | FunASR Model License v1.1 |
| Matcha, Vocos and Zipformer models | See the plugin `licenses/` directory |
| RKNN headers and `librknnrt.so` | Rockchip RKNN SDK license |
