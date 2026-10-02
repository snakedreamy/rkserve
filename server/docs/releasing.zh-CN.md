# 发布流程

[English](releasing.md) | 简体中文

本体和插件在同一仓库中分别构建、发布。

## 本体

`server/Containerfile` 只构建 Core 和控制台，不下载插件模型或 RKNN SDK。

推送 `v*` 标签（例如 `v0.1.0`）会运行 `Image` 工作流，把 `linux/arm64` 镜像发布到 GHCR。手动触发该工作流也会推送镜像。

## 插件

`Plugins` 工作流：

1. 在 x86_64 runner 上按固定输入转换模型。
2. 在 ARM64 runner 上编译 Worker，并按 `plugins/deps.lock` 下载第三方文件。
3. 每个插件打成 `rkserve-plugin-<id>.tar.gz`。
4. 推送 `plugins-v*` 标签时，用这些归档和 `SHA256SUMS` 创建 GitHub Release。

手动运行工作流时，安装包留在 Actions artifacts。已发布的 Release 用 `plugins/install.sh --release` 安装。

不要覆盖已经发布的标签。

## 检查

```bash
python3 -m unittest discover -s plugins/tests -v
shellcheck -x server/deploy/*.sh plugins/install.sh plugins/tools/*.sh plugins/*/build.sh plugins/*/convert/*.sh
(cd server && cargo test --locked -p rkserve-core -p rkserve-protocol)
(cd server/frontend && pnpm test)
```

模型转换和镜像构建在 GitHub 上运行。插件构建完成后，在 RK3576 上安装，确认 Worker 能加载、推理能跑、控制台和 API 鉴权正常。
