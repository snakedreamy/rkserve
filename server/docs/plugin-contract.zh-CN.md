# RKServe Plugin Contract

[English](plugin-contract.md) | 简体中文

## 插件准入

AI Plugin 必须满足：

- 主要模型推理通过 Rockchip NPU Runtime 执行。
- 声明可接受的核心位图、并发、队列和超时。
- 初始化时回报 Runtime、Driver、模型平台和实际核心绑定。
- 提供独立的健康检查、加载、排空和卸载操作。
- 回报排队、NPU 推理、CPU 预处理和后处理耗时。
- 模型、运行库、native 源码和许可证全部位于插件目录，不依赖项目外文件。

纯 CPU 应用不能获得 NPU 租约。

## 自包含目录

安装后的插件包目录如下：

```text
<plugin-root>/<plugin-id>/
  plugin.toml
  bin/worker
  lib/
  assets/
  licenses/
```

`bin/worker` 是插件 `build.sh` 生成的 release ELF；二进制、运行库和模型都不提交到 Git。Worker 使用 `$ORIGIN/../lib` RPATH 加载插件私有库，使用 `RKSERVE_PLUGIN_DIR` 定位模型。

## Manifest v2

```toml
schema_version = 2

[plugin]
id = "yolo26"
name = "YOLO26 Detection"
version = "0.1.0"
protocol_version = 2
executable = "bin/worker"

[[capabilities]]
id = "vision.detect"
name = "Object detection"
description = "Detect objects and return classes, confidence scores and bounding boxes."
input_kind = "image"
accepted_content_types = ["image/jpeg", "image/png"]
max_input_bytes = 10485760
output_kind = "detections"
output_content_type = "application/json"

[resources.npu]
allowed_masks = ["core0", "core1", "core0_1"]
default_mask = "core0_1"
max_concurrency = 1
queue_size = 16
request_timeout_ms = 30000
```

Core 只接受 `schema_version = 2` 和 `protocol_version = 2`。`executable` 必须是插件目录中的相对路径，解析后不得逃逸插件目录。

独占 NPU、必须使用 NPU 和监督重启策略是 Core 固定规则，不在每个 Manifest 中重复声明。Manifest 不支持 `public_path`、任意 HTTP 路由、任意前端脚本或项目外模型路径。

### 显示文本翻译

基础显示字段统一使用英文。可选的顶层翻译表只影响界面文本，不改变协议或配置值：

```toml
[translations.zh-CN]
"plugin.name" = "YOLO26 目标检测"
"capabilities.vision.detect.name" = "目标检测"
"capabilities.vision.detect.description" = "识别图像中的目标并返回类别、置信度和边界框。"
```

键是带引号的**扁平字符串**，能力 ID 中的点号不是 TOML 嵌套层级。API 转发为 `translations: { "zh-CN": { "key": "value" } }`；空表不输出，旧版没有翻译的清单仍可使用。控制台默认英文，支持 `zh-CN`；缺少键时回退到英文基础字段。

支持的键：

- `plugin.name`
- `capabilities.<id>.name` / `.description`
- `capabilities.<id>.presentation.input_hint` / `.input_placeholder`
- `capabilities.<id>.parameters.<key>.label` / `.description`
- `configuration.<key>.label` / `.description`
- 参数或配置键后加 `.options.<raw-value>`，用于选项显示名称。

译文按纯文本显示，不执行 HTML。不得翻译 ID、选项值、默认值、模型路径、renderer 名称或 `text_examples`（这些示例是实际推理输入）。Core 不根据 `Accept-Language` 切换响应语言；错误和事件诊断统一英文。

### 动态发现 RKNN 模型

模型选择字段可以声明 `options_from = "rknn_models"`，无需在 Manifest 或前端写死型号：

```toml
[[configuration.fields]]
key = "model"
label = "Detection model"
description = "Select an RKNN model discovered in the plugin directory."
kind = "select"
required = true
options_from = "rknn_models"
```

Core 启动时递归扫描该插件的 `assets/models/`，只接纳带同目录 `metadata.yaml` 的普通 `.rknn` 文件，忽略符号链接，并将模型根目录内的相对路径作为选项值。默认项按文件大小从小到大选择；配置值仍须属于发现结果，不能传入任意文件路径。新增或移除模型后需要重启 Core 以刷新注册表。

## 能力声明

每个插件至少声明一项能力。Core 以 Manifest 为准完成 HTTP 准入校验，并在 Worker `Describe` 阶段确认 Worker 实际报告了相同能力。

- `id`：传给 `ExecuteJobRequest.capability_id` 的稳定标识。
- `input_kind`：`image`、`audio`、`text` 或 `binary`，用于选择通用输入控件。
- `accepted_content_types`：允许的 HTTP `Content-Type` 白名单。
- `max_input_bytes`：能力载荷上限，不能超过 Core 总上限。
- `output_kind`：`detections`、`audio`、`text`、`json` 或 `binary`，用于选择通用结果呈现器。
- `output_content_type`：成功结果的 MIME 类型。

能力可通过 `[[capabilities.parameters]]` 声明请求参数。字段支持 string、number、boolean 和 select，并可设置默认值、范围与选项。Core 负责校验，控制台生成表单，Worker 从 `ExecuteJobRequest.parameters` 读取解析后的值。

### 声明式运行界面

插件可在能力下声明纯数据展示元数据，由控制台宿主选择受控 renderer：

```toml
[capabilities.presentation]
renderer = "image_detection_overlay"
input_hint = "Upload a JPEG or PNG image; detection boxes will overlay the original."
```

文本能力还可声明 `input_placeholder` 和最多六条 `text_examples`。这些内容始终按普通文本渲染，不解释 HTML 或 Markdown。

允许的 renderer：

- `audio_player`：要求 `output_kind = "audio"` 且结果 MIME 为 `audio/*`，控制台提供播放、重播和下载。
- `image_detection_overlay`：要求 image 输入和 JSON detections 输出，控制台在原图上绘制检测框并保留明细与原始 JSON。
- `image_pose_overlay`：要求 image 输入和 JSON 输出，控制台按 COCO 17 点格式绘制人体边界框、关键点和骨架。
- `speech_transcription`：要求 audio 输入和 JSON 输出，控制台展示转写文本、语言、情绪、声音事件及长音频分段。

`presentation` 是可选提示层；没有声明时，控制台按 `output_kind` 使用默认 renderer。插件不能通过该字段提供 JavaScript、React 模块、HTML、CSS、远程 URL、动态 import 或任意组件名称。

`image_detection_overlay` 使用 RKServe detections v1 结果结构：

```json
{
  "model": "yolo26n",
  "detections": [
    {
      "class_id": 0,
      "label": "person",
      "confidence": 0.92,
      "box": { "left": 16, "top": 24, "right": 300, "bottom": 420 }
    }
  ]
}
```

边界框坐标必须是原输入图像的像素坐标。控制台会裁剪越界坐标并忽略无效框，但插件仍应返回合法结果。

`image_pose_overlay` 使用 RKServe pose v1 结果结构（以下示例缩略了关键点数组）；`keypoints` 必须按 COCO 顺序提供 17 个点，坐标同样使用原输入图像像素坐标：

```json
{
  "model": "yolov8n-pose",
  "poses": [
    {
      "confidence": 0.94,
      "box": { "left": 32, "top": 18, "right": 310, "bottom": 470 },
      "keypoints": [
        { "id": 0, "label": "nose", "x": 172.4, "y": 64.1, "confidence": 0.98 }
      ]
    }
  ]
}
```

`speech_transcription` 使用结构化转写结果。顶层保存合并文本与总体标签，`segments` 保存长音频自动切分后的起止时间和逐段结果：

```json
{
  "text": "开放时间早上九点至下午五点。",
  "language": "zh",
  "emotion": "NEUTRAL",
  "events": ["Speech"],
  "audio_duration_ms": 5500,
  "segments": [
    {
      "index": 0,
      "start_ms": 0,
      "end_ms": 5500,
      "text": "开放时间早上九点至下午五点。",
      "language": "zh",
      "emotion": "NEUTRAL",
      "events": ["Speech"]
    }
  ]
}
```

插件能力只获得异步命名空间：

```text
POST   /api/v1/plugins/{plugin_id}/jobs/{capability_id}
GET    /api/v1/plugins/{plugin_id}/jobs/{job_id}
DELETE /api/v1/plugins/{plugin_id}/jobs/{job_id}
GET    /api/v1/plugins/{plugin_id}/jobs/{job_id}/result
GET    /api/v1/plugins/{plugin_id}/spec
```

`/spec` 由 Core 根据 Manifest 生成，供控制台和外部客户端读取。插件不能覆盖 Core 保留路由。

## 配置声明

配置字段与能力参数使用相同 schema，并通过 `LoadRequest.config` 传给 Worker。停止状态下修改配置只保存新值；运行状态下修改配置会排空并卸载当前 Worker，然后在原核心位图上用新配置重新加载。切换期间新的 Job 可能暂时返回 Worker 不可用。

两者可以声明同一个业务键，但语义不同：`configuration.fields` 是 Worker 加载时使用的实例级默认值，`capabilities.parameters` 是单次 Job 的请求级覆盖。例如 Matcha-TTS 的 `speed` 同时出现在两处，前者控制默认语速，后者允许调用方只为当前任务调整语速；插件实现应在请求未提供覆盖值时回退到实例配置。

配置只能表达用户可调整的运行参数，例如默认语速；模型路径和运行库路径属于插件内部资产，不能配置为项目外路径。

## 启动流程

1. Core 校验 Manifest、Worker 路径和协议版本。
2. NPU 调度页或调度 API申请独占租约。
3. Core 创建独立 Unix Socket 路径，设置 `RKSERVE_PLUGIN_DIR` 并启动 Worker。
4. Worker 使用租约中的确定核心位图和插件内模型初始化 Runtime。
5. Worker 回报 Ready，Core 开始执行该插件的 Job。

Worker 不能自行更改核心位图或启动未声明的模型上下文。

## 异步执行与取消

任务按 `max_concurrency + queue_size` 有界接收，超过容量返回 `429`。获得执行槽后任务进入 `running`；排队任务可以立即取消。原生 NPU 调用不能被安全强制中断时，运行中任务进入 `canceling`，等待当前调用返回后丢弃结果。

Core 到 Worker 使用 gRPC over Unix Domain Socket 的 unary `ExecuteJob`。它是异步 Job 的内部执行原语，不是公开同步推理 API。

## 生命周期

对外状态只暴露实现中真实可观察的值：

```text
installed -> ready
                |
             backoff
                |
             failed
```

更细的启动、加载、排空步骤通过结构化事件观察，不扩展成前端无法稳定消费的瞬时状态。
