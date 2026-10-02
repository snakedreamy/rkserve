# RKServe 系统架构

[English](architecture.md) | 简体中文

## 边界

RKServe 分为控制面和数据面：

- `rkserve-core` 是唯一 HTTP 控制面，负责插件发现、NPU 调度、异步任务、事件、Worker 监督和管理控制台静态文件。
- Plugin Worker 是数据面，独立链接 Rockchip Runtime、持有模型上下文并执行预处理、NPU 推理和后处理。
- Web Console 是普通 API 客户端。所有 `/api/v1/**` 请求都在 Core 使用 Bearer API Key 鉴权；页面是否展示某项操作不是安全授权边界。

```text
External clients / Web Console
              |
        HTTPS / Bearer key
              |
           Caddy
              |
       loopback HTTP
              |
       +------v-------+
       | rkserve-core |
       | API / policy |
       | Job manager  |
       | NPU scheduler|
       | supervisor   |
       +------+-------+
              |
       gRPC over UDS
       +------+------+------+
       |             |      |
  YOLO worker   TTS worker  ...
       |             |
       +------v------+
      librknnrt / RGA
              |
    /dev/dri/renderD129
```

Core 不加载插件共享库，也不接受插件注入任意 HTTP handler 或前端 JavaScript。插件通过 Manifest 声明受控能力，Core 将其暴露在 `/api/v1/plugins/{plugin_id}` 命名空间下。

## 插件包

插件源码位于仓库的 `plugins/` 目录，与 Core 分开。GitHub Actions 的 `Plugins` 工作流从固定版本的上游来源转换各插件模型、编译 Worker，并为每个插件发布一个自包含安装包（GitHub Release 附件）。安装后的插件不依赖自身目录以外的任何文件：

```text
<plugin-root>/<plugin-id>/
  plugin.toml
  bin/worker            # release Worker，RPATH 为 $ORIGIN/../lib
  lib/                  # 插件私有运行库
  assets/               # 模型与数据文件
  licenses/             # 第三方许可证、源码与构建信息
```

源码目录结构以及构建、转换、安装脚本见 [plugins/README.zh-CN.md](../../plugins/README.zh-CN.md)。

## 故障域与恢复

Worker 使用独立 PID、Unix Socket、工作目录和环境变量。Worker 退出后，Core 暂时保留租约并按 1、2、4、8、16 秒指数退避重启，恢复后仍使用原核心。Core 每 5 秒检查 Worker 健康；连续失败后终止 Worker 并进入恢复流程。

连续恢复失败会触发熔断：禁用插件期望状态并释放 NPU 租约，避免故障插件无限重启或永久占核。

## 状态、任务与事件

- `state.json` 保存插件是否应运行及其核心绑定，不保存 PID、Socket 或租约编号。
- `rkserve.sqlite3` 使用 WAL 保存结构化事件和异步 Job 元数据。
- Job 请求和结果采用临时文件加原子 rename 落盘。
- Core 重启后，已完成 Job 的结果仍可读取；未完成 Job 标记为中断失败，不自动重复执行 NPU 请求。
- 事件可按游标、时间、严重程度、类型、插件、租约和 Job 查询。

## NPU 资源模型

RK3576 暴露一个 NPU 设备和两个可调度核心。调度器使用核心位图：

- `core0 = 0b01`
- `core1 = 0b10`
- `core0_1 = 0b11`
- `auto` 表示由 RKServe 选择，不把决策下放给 Worker。

当前只实现独占租约。独占是 Core 的固定调度不变量，不再作为每个插件重复声明的可选字段。

## 调度不变量

1. Worker 在获得租约之前不能初始化 NPU Runtime。
2. 双核租约必须原子分配。
3. 重新绑核必须排空请求并重建 RKNN Context。
4. 队列有界；过载时显式拒绝，不能无界占用内存。
5. 未安装、未运行或不健康的插件不能执行 Job。
6. Worker 只能访问 Core 分配的核心位图，不能自行扩展租约。

## API 边界

系统 API 使用 `/api/v1` 前缀。插件能力只提供异步 Job：

```text
POST   /api/v1/plugins/{plugin_id}/jobs/{capability_id}
GET    /api/v1/plugins/{plugin_id}/jobs/{job_id}
DELETE /api/v1/plugins/{plugin_id}/jobs/{job_id}
GET    /api/v1/plugins/{plugin_id}/jobs/{job_id}/result
GET    /api/v1/plugins/{plugin_id}/spec
```

不存在 Manifest 任意公开路径或同步 `/infer` API。Core 根据 Manifest 的能力、字段和展示元数据生成控制台表单与 API 说明。内部 Worker 协议单独版本化，不对外暴露。

Core 是最终鉴权点，不依赖 Caddy 或控制台替它授权。API Key 按 `read`、`infer`、`control` 和 `audit` scope 最小授权；除健康检查外没有匿名 API。Caddy 只负责 HTTPS 和反向代理，Core 默认监听回环地址并默认不信任转发头。

## 语言边界

Core 提供英文基础显示字段和可选 `translations` 映射。控制台只翻译显示文本，不修改 ID 或配置值；默认英文，可保存 `zh-CN` 选择。诊断和事件保持英文，用户输入和模型输出原样保留。
