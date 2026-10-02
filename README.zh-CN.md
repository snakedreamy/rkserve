# RKServe

[English](README.md) | 简体中文

[![CI](https://github.com/snakedreamy/rkserve/actions/workflows/ci.yml/badge.svg)](https://github.com/snakedreamy/rkserve/actions/workflows/ci.yml)
[![Image](https://github.com/snakedreamy/rkserve/actions/workflows/image.yml/badge.svg)](https://github.com/snakedreamy/rkserve/pkgs/container/rkserve)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

RKServe 是面向 Rockchip RK35xx 边缘设备的 NPU 服务平台。`rkserve-core` 负责 NPU 调度、插件生命周期、异步 Job、结构化事件和统一 HTTP 边界；算法作为独立进程插件运行，崩溃和 native 依赖不会进入 Core 进程。

## 设计原则

- Core 不链接 `librknnrt`，Worker 独立持有模型上下文。
- 插件必须显式声明可用核心、并发、队列和超时。
- 安装后的插件自包含：Worker、模型、运行库和许可证都在插件自己的目录内。
- 对外推理只提供异步 Job，不保留同步 `/infer` 或任意公开路径。
- Core 保留唯一 HTTP 信任边界，插件不能注入任意服务端路由或前端 JavaScript。
- Core 对全部 `/api/v1/**` 请求强制执行 Bearer API Key 鉴权；控制台也是普通、非特权 API 客户端。

## 语言

控制台默认使用 **English**，登录前后均可切换为**简体中文**。语言偏好保存在本机；浏览器语言不会覆盖默认值。插件说明、表单、状态和日期/数字格式随语言切换；缺少译文时回退到英文。

项目文档提供配套 `.zh-CN.md`。项目维护的代码注释、服务诊断和脚本输出统一使用英文。API 标识符、模型路径、用户输入、识别结果、模型词表及第三方法律文本不翻译。

## 插件

Core 本身不带插件。按需从 GitHub Release 安装即可。模型均针对 **RK3576** 转换；其他 RK35xx 芯片需要自行转换。

| 插件 | 能力 | 模型 |
| --- | --- | --- |
| `yolo26` | 目标检测 | Ultralytics YOLO26 n/s/m/l/x（FP16，640） |
| `yolov8-pose` | 人体姿态估计 | YOLOv8n-Pose |
| `sensevoice-asr` | 多语言语音理解（文本、语言、情绪、事件） | SenseVoice-small |
| `zipformer-asr` | 中英文语音识别 | Zipformer bilingual zh-en |
| `matcha-tts` | 中英文语音合成 | Matcha-TTS + Vocos |

```bash
plugins/install.sh --dest "$HOME/rkserve/plugins" yolo26 sensevoice-asr
```

安装脚本从 GitHub Release 下载 `rkserve-plugin-*.tar.gz` 并校验 SHA-256。本地归档可用 `--from DIR`。模型和随附第三方组件保留各自许可证（包括 AGPL-3.0、GPL-3.0 和 Rockchip RKNN SDK 许可），详见 [plugins/README.zh-CN.md](plugins/README.zh-CN.md)。

## 目录

```text
server/                 RKServe 本体（Apache-2.0）
  core/                 HTTP API、调度、Job、事件和 Worker 监督
  protocol/             Core/Worker Protobuf 协议
  frontend/             React 管理控制台
  deploy/               镜像构建、Quadlet、Caddy 与 API Key 示例
  docs/                 架构、插件契约、HTTP API 和部署文档
  Containerfile         Core + 控制台镜像，不含插件

plugins/                插件源码、模型转换与打包
  <plugin-id>/          Manifest、Worker、转换脚本和许可证
  tools/                共用的构建、依赖下载和打包脚本
  deps.lock             下载的第三方文件的官方地址与 SHA-256
  install.sh            插件安装脚本
```

Git 仓库不提交模型权重、编译后的 Worker 或下载的运行库；小型数据文件和控制台字体仍保留。

## 快速开始：镜像与插件

发布的 `linux/arm64` 镜像只包含 Core 和控制台。插件安装在宿主机上，再挂载进容器。

```bash
podman pull ghcr.io/snakedreamy/rkserve:latest
plugins/install.sh --dest "$HOME/rkserve/plugins" yolo26
```

创建 API Key secret，映射 NPU/RGA 设备，并把插件目录挂载到 `/opt/rkserve/plugins`。完整命令见[安全与 Caddy 部署](server/docs/security-and-deployment.zh-CN.md)；长期运行可使用 [Rootless Quadlet 部署](server/deploy/quadlet/README.zh-CN.md)。

## 从源码构建

在设备上从仓库根目录构建 Core 和控制台：

```bash
(cd server && cargo build --locked --release -p rkserve-core)
(cd server/frontend && corepack enable pnpm && pnpm install --frozen-lockfile && pnpm run build)
```

插件从 GitHub Release 安装。如需自行构建，见 [plugins/README.zh-CN.md](plugins/README.zh-CN.md)。

## 运行

在仓库根目录创建密钥文件，并使用已安装的插件启动 Core：

```bash
install -m 600 server/deploy/api-keys.example.json server/deploy/api-keys.json
# 使用 openssl rand -hex 32 生成密钥并替换示例占位值

RKSERVE_LISTEN=127.0.0.1:8080 \
RKSERVE_API_KEYS_FILE=server/deploy/api-keys.json \
RKSERVE_PLUGIN_ROOT="$HOME/rkserve/plugins" \
RKSERVE_RUNTIME_ROOT=run/plugins \
RKSERVE_STATE_ROOT=run \
RKSERVE_CONSOLE_ROOT=server/frontend/dist \
  server/target/release/rkserve-core
```

缺少有效的 `RKSERVE_API_KEYS_FILE` 时 Core 会拒绝启动。默认只监听 `127.0.0.1:8080`；对外部署建议由 Caddy 提供 HTTPS，详见[安全与 Caddy 部署](server/docs/security-and-deployment.zh-CN.md)。Core 使用：

- `RKSERVE_RUNTIME_ROOT`：Unix Socket、Worker 标记和短生命周期文件；
- `RKSERVE_STATE_ROOT/state.json`：插件期望状态与核心绑定；
- `RKSERVE_STATE_ROOT/rkserve.sqlite3`：事件与 Job 元数据；
- `RKSERVE_STATE_ROOT/jobs/`：Job 请求和结果文件。

这些运行时文件均被 `.gitignore` 排除。

浏览器通过配置好的 HTTPS 域名进入控制台，首次访问需输入部署时分配的 API Key。模型选择位于“插件 → YOLO26 Detection → 启动配置”，Core 会列出插件 `assets/models/` 下自动发现的模型。

## 控制台页面

- `/npu`：NPU 拓扑、核心分配，以及唯一的插件启停入口。
- `/plugins`：只读插件清单、状态和能力。
- `/plugins/:id`：插件配置、资源和 Runtime 状态；不提供启停。
- `/plugins/:id/run`：提交、轮询、取消异步 Job 并读取结果。
- `/apis`：Manifest 驱动的异步 API 说明。
- `/events`：按游标和结构化字段查询运行事件。

## 异步调用示例

以下示例先设置密钥：

```bash
export RKSERVE_API_KEY='<部署时生成的密钥>'
```

先在 NPU 页面启动 YOLO26，或调用调度 API：

```bash
curl -X POST http://127.0.0.1:8080/api/v1/scheduler/allocations \
  -H "Authorization: Bearer ${RKSERVE_API_KEY}" \
  -H 'Content-Type: application/json' \
  -d '{"plugin_id":"yolo26","core_mask":"core0_1"}'
```

提交目标检测 Job：

```bash
curl -X POST \
  http://127.0.0.1:8080/api/v1/plugins/yolo26/jobs/vision.detect \
  -H "Authorization: Bearer ${RKSERVE_API_KEY}" \
  -H 'Content-Type: image/jpeg' \
  --data-binary '@image.jpg'
```

SenseVoice 多语言语音理解返回结构化 JSON，包含文本、语言、情绪、声音事件与长音频分段：

```bash
curl -X POST \
  'http://127.0.0.1:8080/api/v1/plugins/sensevoice-asr/jobs/audio.transcribe?language=auto&text_normalization=withitn' \
  -H "Authorization: Bearer ${RKSERVE_API_KEY}" \
  -H 'Content-Type: audio/wav' \
  --data-binary '@speech.wav'
```

Zipformer 中英文语音识别同样使用异步 Job；默认搜索宽度 `4` 以识别效果优先：

```bash
curl -X POST \
  'http://127.0.0.1:8080/api/v1/plugins/zipformer-asr/jobs/audio.transcribe?beam_size=4' \
  -H "Authorization: Bearer ${RKSERVE_API_KEY}" \
  -H 'Content-Type: audio/wav' \
  --data-binary '@speech.wav'
```

使用返回的 `id` 替换 `JOB_ID` 查询状态和结果：

```bash
curl -H "Authorization: Bearer ${RKSERVE_API_KEY}" \
  http://127.0.0.1:8080/api/v1/plugins/yolo26/jobs/JOB_ID
curl -o result.json \
  -H "Authorization: Bearer ${RKSERVE_API_KEY}" \
  http://127.0.0.1:8080/api/v1/plugins/yolo26/jobs/JOB_ID/result
```

## 文档

- [系统架构](server/docs/architecture.zh-CN.md)
- [插件契约](server/docs/plugin-contract.zh-CN.md)
- [HTTP API](server/docs/api-reference.zh-CN.md)
- [安全与 Caddy 部署](server/docs/security-and-deployment.zh-CN.md)
- [插件：安装、构建与许可证](plugins/README.zh-CN.md)
- [发布流程](server/docs/releasing.zh-CN.md)

## 许可证

`server/` 中的代码和 `plugins/` 中的插件源码以 [Apache License 2.0](LICENSE) 发布，`matcha-tts` Worker 例外，使用 GPL-3.0-only。插件安装包随附的第三方模型和运行库保留各自许可证，详见 [plugins/README.zh-CN.md](plugins/README.zh-CN.md)。
