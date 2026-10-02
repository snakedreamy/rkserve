# RKServe 插件

[English](README.md) | 简体中文

插件源码、第三方许可、模型转换及打包工具集中在此目录，与 `server/` 本体分开。

## 安装

插件包包含 `plugin.toml`、`bin/worker`、`lib/`、`assets/` 和许可证文本。使用者不用编译。转换工作区 `licenses/model-sources/`（原始权重、wheels、中间文件）留在转换产物里，不进入 GitHub Release 安装包。安装需要 Bash、Python 3、curl、tar、sha256sum 和 flock；运行需要匹配的 ARM64 系统、RK3576 及宿主 NPU 驱动。

```bash
plugins/install.sh --dest "$HOME/rkserve/plugins" yolo26 sensevoice-asr
```

安装脚本从 GitHub Release 下载 `rkserve-plugin-*.tar.gz`（默认标签 `plugins-v0.1.0`）并校验 SHA-256。本地归档可用 `--from DIR`；目录中须有这些归档，以及 `SHA256SUMS` 或 `package.sh` 生成的 `*.tar.gz.sha256`。更新时先停止 RKServe，安装完成后重启。插件是本地原生程序，不是安全沙箱，只安装可信来源的包。

## 源码与构建

- `<插件>/worker/`：Rust Worker 与 C/C++ bridge。
- `<插件>/convert/`：模型转换步骤与来源记录。
- `<插件>/licenses/`：组件许可证和来源说明。
- `vendor/`：多个插件共用的第三方源码。
- `deps.lock`：第三方头文件和运行库的固定下载地址与 SHA-256。
- `tools/`：下载、构建、打包脚本。

`server/` 与 `plugins/` 使用独立 Cargo workspace，插件引用本仓库的 `server/protocol`。Core 不链接插件、RKNN 或模型。

插件 Worker 仍需由发布者编译；用户安装的是编译后的包。ARM64 编译和 x86_64 模型转换由 GitHub Actions 完成。手动构建入口为：

```bash
plugins/yolo26/build.sh /absolute/path/to/worker-output
```

下载依赖保存在被 Git 忽略的 `.deps/`。不要把该目录或转换工作目录提交到仓库。

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
