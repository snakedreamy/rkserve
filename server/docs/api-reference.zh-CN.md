# HTTP API 参考

[English](api-reference.md) | 简体中文

RKServe 默认监听 `127.0.0.1:8080`。除 `/health` 外，接口位于 `/api/v1`。

生产环境由 Caddy 提供 HTTPS，但全部 `/api/v1/**` 请求仍由 Core 强制鉴权。控制台没有特殊信任。

## 通用约定

- 请求必须包含 `Authorization: Bearer <API Key>` 或 `X-API-Key: <API Key>`；同时提供时以 Bearer 为准。密钥只允许放在 Header，不能放进 URL。
- 密钥分 `management`（控制台）和 `api`（推理接口）。接口密钥即使认证成功，访问控制台只读/调度/审计接口也会返回 `403`。
- JSON 接口使用 `application/json`。
- Job 请求体是原始二进制或 UTF-8 文本，`Content-Type` 必须匹配能力声明。
- 单个请求体受 Core 总上限和 capability `max_input_bytes` 双重限制。
- 响应包含 `X-Request-ID`；客户端也可提交 1–64 字符的安全请求 ID 以便端到端关联。
- 错误响应格式为 `{ "error": "Error description" }`；鉴权失败还包含 `request_id`。API 标识符、配置值和诊断/事件消息不随界面语言改变，英文原文用于统一排错。

scope 与接口：

| scope | 接口 |
|---|---|
| `read` | 系统、遥测、拓扑、插件、Worker、租约查询 |
| `infer` | Job 提交、状态、取消与结果 |
| `control` | 插件配置、租约创建与释放 |
| `audit` | 事件日志 |
| `*` | 全部 |

常见状态码：

| 状态码 | 含义 |
|---|---|
| `401` | 缺少或无法识别 API Key |
| `403` | API Key 缺少所需 scope 或角色不允许 |
| `400` | 请求参数无效 |
| `404` | 插件、能力、租约或 Job 不存在 |
| `409` | 核心繁忙、插件状态冲突或结果未就绪 |
| `413` | 请求体超过能力限制 |
| `415` | `Content-Type` 不受支持 |
| `422` | 核心掩码、Manifest 或 Worker 能力不匹配 |
| `429` | Job 队列已满 |
| `500` | 状态、数据库或结果持久化失败 |
| `503` | Worker 启动、通信、执行或停止失败 |

## 系统与插件

### `GET /health`

```json
{ "status": "ok" }
```

唯一匿名端点，用于 Caddy/容器健康检查。

### `GET /api/v1/auth/whoami`

返回当前 API Key 的身份名称、scope 与不可逆指纹；控制台用它验证用户输入的密钥。

### `GET /api/v1/system`

返回服务版本、平台和已发现插件数量。

### `GET /api/v1/system/telemetry`

返回设备健康遥测：SoC/NPU 温度、总内存、可用内存，以及驱动能够提供时的 NPU 分核负载。温度通过 thermal zone 的 `type` 匹配，不依赖固定编号；分核负载来自可选的 debugfs 节点。无法读取的字段返回 `null`，不会使接口失败。

### `GET /api/v1/npu/topology`

返回 RK3576 NPU 拓扑、频率、负载、核心及当前租约。

### `GET /api/v1/plugins`

返回插件 Manifest 摘要、能力、配置、解析值、运行状态和可选的 `translations` 显示译文。

### `GET /api/v1/plugins/{plugin_id}`

返回指定插件的完整摘要。

### `GET /api/v1/plugins/{plugin_id}/spec`

返回 Core 根据 Manifest v2 生成的结构化异步 API、输入字段、输出类型和示例说明。控制台 API 页面使用同一份数据。

### `GET /api/v1/workers`

返回 Worker 快照。Worker 状态为 `ready` 或 `backoff`。

## NPU 租约

### `GET /api/v1/scheduler/allocations`

返回当前租约列表。

### `POST /api/v1/scheduler/allocations`

```json
{
  "plugin_id": "yolo26",
  "core_mask": "core0_1"
}
```

`core_mask` 可为 `auto`、`core0`、`core1` 或 `core0_1`。成功返回 `201` 和创建的租约。

### `DELETE /api/v1/scheduler/allocations/{lease_id}`

停止对应 Worker 并释放租约。成功返回 `204`。

## 插件配置

### `PUT /api/v1/plugins/{plugin_id}/configuration`

插件停止时，配置会保存并在下次启动时读取。插件运行时，Core 会排空并卸载当前 Worker，再使用原核心位图和新配置重新加载。模型和运行库路径不能指向插件目录之外；支持模型发现的插件只接受 Core 已在自身 `assets/models/` 中发现的相对路径。

```json
{
  "values": {
    "model": "custom_people_640/custom-people-rk3576.rknn"
  }
}
```

成功返回解析默认值后的完整配置对象。

## 异步 Job

RKServe 不提供公开同步推理或 Manifest 自定义公开路径。所有能力统一使用 Job API。

### `POST /api/v1/plugins/{plugin_id}/jobs/{capability_id}`

查询参数作为 capability 参数，请求体和 `Content-Type` 按 Manifest 声明传入。成功返回 `202` 和 Job 快照。

```bash
curl -X POST \
  'http://127.0.0.1:8080/api/v1/plugins/yolo26/jobs/vision.detect' \
  -H "X-API-Key: ${RKSERVE_API_KEY}" \
  -H 'Content-Type: image/jpeg' \
  --data-binary '@image.jpg'
```

Job 状态包括 `queued`、`running`、`canceling`、`succeeded`、`failed` 和 `canceled`。

### `GET /api/v1/plugins/{plugin_id}/jobs/{job_id}`

返回 Job 状态、时间、取消状态、结果大小、分阶段耗时和错误。路径中的 `plugin_id` 必须与 Job 所属插件一致。

### `DELETE /api/v1/plugins/{plugin_id}/jobs/{job_id}`

请求取消 Job。排队任务立即取消；无法中断的原生 NPU 调用会先进入 `canceling`，在当前执行安全结束后丢弃结果。

### `GET /api/v1/plugins/{plugin_id}/jobs/{job_id}/result`

仅当 Job 为 `succeeded` 时返回结果，否则返回 `409`。响应 `Content-Type` 来自 capability 声明，并包含可用的分阶段耗时头。

## 结构化事件

### `GET /api/v1/events`

事件按倒序分页，响应：

```json
{
  "items": [],
  "next_cursor": null
}
```

支持的查询参数：

| 参数 | 说明 |
|---|---|
| `cursor` | 下一页游标 |
| `after` | 只返回指定事件 ID 之后的新事件 |
| `limit` | 单页数量 |
| `severity` | `info`、`warning` 或 `error` |
| `category` | 事件分类 |
| `kind` | 事件类型 |
| `plugin_id` | 插件 ID |
| `lease_id` | 租约 ID |
| `job_id` | Job ID |
| `request_id` | HTTP 请求 ID |
| `actor` | API Key 身份名称 |
| `client_ip` | 解析后的客户端 IP |
| `http_method` | HTTP 方法 |
| `http_status` | HTTP 状态码 |
| `source` | 事件来源，接口审计为 `http` |
| `exclude_source` | 排除来源，运行事件为 `http` |
| `q` | 搜索消息、路径、请求 ID 或调用方，最多 64 字符 |
| `outcome` | `success` 或 `failure` |
| `path_prefix` | 路径前缀，例如 `/api/v1/plugins` |
| `from` / `to` | Unix 毫秒时间范围 |

事件包含 plugin/lease/job/request ID、调用身份、客户端/直连 IP、HTTP 方法/路径/状态、耗时、请求与响应字节数、source 和结构化 metadata。控制台通过 `after` 增量刷新，通过 `cursor` 加载历史事件。

Core 不保存 API Key、查询参数值或原始请求/响应内容。Job 事件使用 SHA-256 关联输入输出；失败请求、鉴权失败、状态变更、whoami、插件 spec 和结果下载会进入持久化审计（`source=http`），控制台高频成功轮询不会逐条写库。默认保留 30 天且最多 100,000 条，可通过 `RKSERVE_EVENT_RETENTION_DAYS` 和 `RKSERVE_EVENT_MAX_ROWS` 调整。
