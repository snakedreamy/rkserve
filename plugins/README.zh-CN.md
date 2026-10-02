# RKServe 插件

[English](README.md) | 简体中文

插件源码、第三方许可、模型转换及打包工具集中在此目录，与 `server/` 本体分开。GitHub Actions 已完成五个插件的转换、Worker 构建和打包；GitHub Release 尚未发布。

## 安装

插件包包含 `plugin.toml`、`bin/worker`、`lib/`、`assets/` 和 `licenses/`。使用者不用编译。安装需要 Bash、Python 3、curl、tar、sha256sum 和 flock；运行需要匹配的 ARM64 系统、RK3576 及宿主 NPU 驱动。

```bash
plugins/install.sh --from /path/to/packages --dest "$HOME/rkserve/plugins" yolo26 sensevoice-asr
```

`--from` 目录中须有 `rkserve-plugin-*.tar.gz`，以及 `SHA256SUMS` 或 `package.sh` 生成的 `*.tar.gz.sha256`。在发布 `plugins-v*` 标签之后，也可用 `--release` 从 GitHub Release 下载。更新时先停止 RKServe，安装完成后重启。插件是本地原生程序，不是安全沙箱，只安装可信来源的包。

## 源码与构建

- `<插件>/worker/`：Rust Worker 与 C/C++ bridge。
- `<插件>/convert/`：模型转换步骤与来源记录。
- `<插件>/licenses/`：组件许可证和来源说明。
- `vendor/`：多个插件共用的第三方源码。
- `deps.lock`：第三方头文件和运行库的固定下载地址与 SHA-256。
- `tools/`：下载、构建、打包脚本。

`server/` 与 `plugins/` 使用独立 Cargo workspace，插件引用本仓库的 `server/protocol`。Core 不链接插件、RKNN 或模型。

插件 Worker 仍需由发布者编译；用户安装的是编译后的包。预期由 GitHub Actions 完成 ARM64 编译和 x86_64 模型转换，本地无需全量编译。手动构建入口为：

```bash
plugins/yolo26/build.sh /absolute/path/to/worker-output
```

下载依赖保存在被 Git 忽略的 `.deps/`。不要把该目录或转换工作目录提交到仓库。GitHub Actions 上的模型转换、ARM64 Worker 构建和打包已经通过。新模型的板端推理，以及公开的插件 Release，是另一步。

## 许可证

各插件的许可文本在 `<插件>/licenses/`。简要如下：

| 组件 | 许可 |
| --- | --- |
| 大部分自有 Worker 源码 | Apache-2.0 |
| Matcha Worker、eSpeak NG | GPL-3.0 |
| YOLO26、YOLOv8-Pose 模型 | AGPL-3.0 |
| SenseVoice 模型 | FunASR Model License v1.1 |
| Matcha、Vocos、Zipformer 模型 | 见该插件 `licenses/` |
| RKNN 头文件和 `librknnrt.so` | Rockchip RKNN SDK 许可 |
