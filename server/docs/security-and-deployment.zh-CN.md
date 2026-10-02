# 安全与 Caddy 部署

[English](security-and-deployment.md) | 简体中文

## 信任边界

- `/health` 和控制台静态文件公开；所有 `/api/v1/**` 请求（包括控制台发起的请求）都由 `rkserve-core` 验证 API Key。支持 `Authorization: Bearer <key>` 或 `X-API-Key: <key>`，同时提供时以 Bearer 为准。
- Caddy 负责 TLS 终止，但不是授权边界。即使绕过控制台直接调用 Core，仍必须通过 Core 鉴权。
- 默认只监听 `127.0.0.1:8080`。不要在 Caddy 前置时把 Core 的端口发布到局域网或公网。
- RKServe 默认忽略 `X-Forwarded-For`。只有直连来源 IP 位于 `RKSERVE_TRUSTED_PROXIES` 时才采用其第一个地址；转发地址只用于审计日志，不参与授权决策。

## 创建 API Key

在仓库根目录复制示例并生成两把独立密钥：管理密钥登录控制台，接口密钥只调用推理 Job。

```bash
install -m 600 server/deploy/api-keys.example.json server/deploy/api-keys.json
RKSERVE_KEY="$(openssl rand -hex 32)"
RKSERVE_API_KEY="$(openssl rand -hex 32)"
sed -i "s/REPLACE_WITH_A_RANDOM_KEY_OF_AT_LEAST_32_BYTES/${RKSERVE_KEY}/" server/deploy/api-keys.json
sed -i "s/REPLACE_WITH_A_SECOND_RANDOM_KEY_OF_AT_LEAST_32_BYTES/${RKSERVE_API_KEY}/" server/deploy/api-keys.json
printf '控制台管理密钥：%s\n' "${RKSERVE_KEY}"
printf '推理接口密钥：%s\n' "${RKSERVE_API_KEY}"
unset RKSERVE_KEY RKSERVE_API_KEY
```

真实的 `server/deploy/api-keys.json` 已被 `.gitignore` 排除。Core 会拒绝以下配置：文件不存在、符号链接、组或其他用户可读写、密钥少于 32 字节、占位密钥、重复名称/密钥、未知 scope、role 与 scope 不匹配，或没有任何启用中的密钥。密钥项可设 `"enabled": false` 停用，无需删除条目；停用的密钥仍走同样的格式校验，且不能与其他密钥重复。密钥在启动时加载，修改文件后需重启 Core。

密钥角色：

| role | 用途 |
|---|---|
| `management` | 管理密钥。登录控制台，访问系统、插件、调度和审计。可同时授予 `infer` 以便在控制台试跑。 |
| `api` | 接口密钥。只能调用 `/api/v1/auth/whoami` 和插件 Job 接口；不能登录控制台，也不能读审计或改调度。 |

可用 scope：

| scope | 能力 |
|---|---|
| `read` | 系统、NPU、插件、Worker 和租约只读查询 |
| `infer` | 提交、查询、取消 Job 和下载结果 |
| `control` | 修改插件配置、创建和释放 NPU 租约 |
| `audit` | 查询事件与审计日志 |
| `*` | 所有能力；仅建议用于受控管理密钥 |

## 安装插件

Core 本身不带插件。按需从插件 Release 安装到插件根目录，每个安装包都会按 Release 中的 `SHA256SUMS` 校验：

```bash
plugins/install.sh --dest "$HOME/rkserve/plugins" yolo26 sensevoice-asr
```

更新插件时重新执行同一命令，然后重启 Core。可用插件及其许可证见 [plugins/README.zh-CN.md](../../plugins/README.zh-CN.md)。

## 启动 RKServe

```bash
RKSERVE_LISTEN=127.0.0.1:8080 \
RKSERVE_API_KEYS_FILE="$PWD/server/deploy/api-keys.json" \
RKSERVE_TRUSTED_PROXIES=127.0.0.1,::1 \
RKSERVE_EVENT_RETENTION_DAYS=30 \
RKSERVE_EVENT_MAX_ROWS=100000 \
RKSERVE_PLUGIN_ROOT="$HOME/rkserve/plugins" \
RKSERVE_RUNTIME_ROOT=run/plugins \
RKSERVE_STATE_ROOT=run \
RKSERVE_CONSOLE_ROOT=server/frontend/dist \
  server/target/release/rkserve-core
```

如果 Caddy 与 RKServe 都以容器运行，推荐把二者放进同一个 Podman pod，共享网络命名空间并继续使用 `127.0.0.1:8080`。不要为了方便把 Core 映射为宿主机的 `0.0.0.0:8080`。

## 在 ARM 设备上使用 rootless Podman 构建

`server/Containerfile` 从仓库根目录构建，分三个阶段：

1. Node builder 生成控制台静态文件；
2. Rust builder 编译 Core；
3. 最终 Ubuntu 24.04 镜像只复制 Core、控制台和项目许可证。

镜像不包含任何插件、模型或 RKNN 运行库；插件安装在宿主机上，以只读方式挂载到 `/opt/rkserve/plugins`。Worker 计划在 Debian 11 中编译；整个插件包（包括下载的运行库）的最低 glibc 要求及镜像兼容性仍需验证。Rust、Node、GCC 和 JavaScript 包管理器不会安装到宿主，也不会进入最终镜像。构建缓存和基础镜像只保存在当前普通用户的 Podman storage 中。前端 Builder 固定使用 Vite 8 支持的 Node 20.20.2 和 pnpm 10.34.5，并通过 `pnpm-lock.yaml` 冻结依赖；这避开了 npm 在 ARM 容器内可能报错却返回成功状态的问题。

宿主机只需要：

- aarch64 Armbian；
- rootless Podman（推荐默认的 `crun` OCI runtime）；
- Python 3、`curl`、`tar`、`sha256sum`，用于下载并校验插件安装包；
- 当前普通用户具有 `render`、`video` 补充组。

首次配置设备组可能需要管理员执行一次：

```bash
sudo usermod -aG render,video "$USER"
```

随后注销并重新登录，确认 `id -nG` 同时包含 `render` 和 `video`。编译和日常运行都不要再使用 `sudo`。

构建：

```bash
./server/deploy/build-container.sh localhost/rkserve:latest
```

脚本会拒绝 root 用户、非 aarch64 主机和非 rootless Podman。可在内存较少的设备上降低并行度：

```bash
RKSERVE_BUILD_JOBS=2 ./server/deploy/build-container.sh localhost/rkserve:latest
```

Podman 默认把宿主的 `http_proxy`、`https_proxy` 等变量传给 Builder。如果脚本发现代理指向 `127.0.0.1`、`localhost` 或 `[::1]`，会仅为镜像构建的 `RUN` 步骤自动选择 host network，让 Builder 能连接宿主回环代理。该模式会扩大构建期间的本地网络可见范围，但不会写入最终镜像或改变运行容器的网络配置；前端依赖安装同时禁用了 lifecycle scripts。未使用回环代理时仍使用 Podman 默认的隔离网络。也可以用 `RKSERVE_BUILD_NETWORK=private` 禁止自动切换，或显式指定 Podman 支持的其他 build network mode。

根目录的 `.dockerignore`（Podman 同样读取）只允许 Core、协议、控制台源码和许可证文件进入构建上下文；构建产物、`node_modules`、插件和 Git 数据都不会发送进去。

构建输出使用 Podman 支持的 Docker v2 image format，以保留 Containerfile 中的健康检查元数据。构建不会修改宿主的软件环境，但 Podman 镜像和 builder layer 会占用当前用户的容器存储。使用 `podman system df` 可以查看占用；不要在不了解影响时执行会清理其他项目缓存的全局 prune。

## 创建 Podman secret

可以不在磁盘上创建明文 JSON，直接把随机密钥写入 rootless Podman secret：

```bash
RKSERVE_KEY="$(openssl rand -hex 32)"
printf '{"keys":[{"name":"console","key":"%s","scopes":["read","infer","control","audit"]}]}\n' \
  "${RKSERVE_KEY}" |
  podman secret create rkserve-api-keys -
printf '请立即保存到密码管理器，控制台 API Key：%s\n' "${RKSERVE_KEY}"
unset RKSERVE_KEY
```

密钥不会写入镜像、Git 或容器环境变量。运行时以 UID 0、GID 0、`0400` 文件挂载到 `/run/secrets/rkserve-api-keys`。

## 以容器内 root、宿主 rootless 方式运行

以下示例使用本地构建的 `localhost/rkserve:latest`。在 GitHub Actions 已成功发布镜像后，也可以使用预构建镜像：先执行 `podman pull ghcr.io/snakedreamy/rkserve:latest`，再把命令中的镜像名替换为 `ghcr.io/snakedreamy/rkserve:latest`。

以下示例在宿主运行 Caddy，RKServe 使用 host network 但只监听回环地址。[Quadlet 指南](../deploy/quadlet/README.zh-CN.md)提供独立网络命名空间方案。先创建 Podman 管理的持久卷，并把插件安装到宿主机目录：

```bash
podman volume create rkserve-state
plugins/install.sh --dest "$HOME/rkserve/plugins" yolo26
```

然后以普通用户启动：

```bash
podman run -d \
  --name rkserve \
  --network host \
  --init \
  --read-only \
  --tmpfs /run/rkserve:rw,nosuid,nodev,noexec,size=64m,mode=0700 \
  --tmpfs /tmp:rw,nosuid,nodev,noexec,size=64m,mode=1777 \
  --volume rkserve-state:/var/lib/rkserve:rw \
  --volume "$HOME/rkserve/plugins:/opt/rkserve/plugins:ro" \
  --secret rkserve-api-keys,type=mount,target=/run/secrets/rkserve-api-keys,uid=0,gid=0,mode=0400 \
  --device /dev/dri/renderD129 \
  --device /dev/rga \
  --device /dev/dma_heap/system \
  --device /dev/dma_heap/system-uncached \
  --group-add keep-groups \
  --cap-drop all \
  --security-opt no-new-privileges \
  --pids-limit 256 \
  --env RKSERVE_LISTEN=127.0.0.1:8080 \
  --env RKSERVE_TRUSTED_PROXIES=127.0.0.1,::1 \
  localhost/rkserve:latest
```

如果设备没有某个 `/dev/dma_heap/*` 节点，可删除对应参数；`renderD129` 是 RK3576 NPU 的必要设备，`/dev/rga` 用于图像预处理加速。

这里的容器 UID 0 默认映射为启动 Podman 的宿主普通用户，而不是宿主 UID 0。`--group-add keep-groups` 保留宿主用户的 `render`/`video` 补充组以通过设备 DAC 检查；该功能要求 `crun`。容器同时删除全部 capabilities、启用只读根文件系统和 `no-new-privileges`，因此不需要也不应使用 `--privileged`。

在 SELinux 主机上，如果设备访问仍被策略拒绝，应由管理员按 Podman 文档启用 `container_use_devices`，不要直接改用 `--privileged`。Armbian/Ubuntu 通常使用 AppArmor，不需要这个 SELinux 设置。

宿主 Caddy 继续代理 `127.0.0.1:8080`。不要把 RKServe 的 8080 端口发布到局域网；如果后续将 Caddy 也容器化，应让两者共享同一个 Podman pod，并且只发布 Caddy 的 HTTP/HTTPS 端口。

NPU 推理本身不依赖 debugfs。rootless 容器默认不会获得 `/sys/kernel/debug/rknpu/load` 的宿主 root 读取权限，因此控制台中的 NPU 分核瞬时负载可能显示为不可用；这不影响模型加载、推理或整体 devfreq 指标。不要为了一项监控数据挂载整个 debugfs 或改用特权容器。

常用检查：

```bash
podman healthcheck run rkserve
podman logs rkserve
curl http://127.0.0.1:8080/health
```

## Caddy

将 [`deploy/Caddyfile.example`](../deploy/Caddyfile.example) 复制到 Caddy 配置目录，替换域名。示例执行以下工作：

- 由 Caddy 自动申请并续期 HTTPS 证书；
- 覆盖转发头，防止客户端自行提交伪造值；
- 将请求体限制为与 Core 一致的 64 MiB；
- 增加 HSTS、CSP、禁止 MIME 嗅探和禁止嵌入等浏览器安全响应头。

Caddy 会原样转发 `Authorization` 和 `X-API-Key`，API Key 最终仍由 RKServe 验证。不要在 Caddy access log 中记录这些请求头。

## 控制台密钥行为

控制台没有用户名/密码登录，也不签发 Cookie。用户首次访问时手动输入 API Key，浏览器只在当前标签页的 `sessionStorage` 中保存；刷新页面仍可使用，关闭标签页后清除。不要把 API Key 写进前端构建产物、URL 查询参数、Caddyfile 或 Git 仓库。语言偏好不是秘密，单独保存在 `localStorage`。

## 审计数据

审计日志记录调用身份、密钥指纹、客户端/直连 IP、请求 ID、方法、路径、状态、耗时、请求/响应字节数、媒体类型以及 plugin/job 关联。Job 还记录输入和输出的 SHA-256。

默认不记录 API Key、请求查询值、原始文本、图片、音频或推理结果。控制台高频只读轮询（topology、telemetry、workers、allocations、plugins、events）成功时不逐条写库；失败请求、鉴权失败、whoami、插件 spec、状态变更、Job 提交和结果下载会持久化，`source` 为 `http`。控制台「事件日志」用接口审计查看这些调用，用运行事件查看调度与 Worker 生命周期。默认保留 30 天且最多 100,000 条，以先达到的限制为准。两种控制台语言均原样显示英文诊断消息。
