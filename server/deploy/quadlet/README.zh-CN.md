# RKServe Rootless Quadlet 部署

[English](README.md) | 简体中文

本方案适配 Podman 5.4.2 及以上版本，使用用户级 Quadlet 管理容器。编译用户与运行用户的 Rootless Podman storage 相互隔离，因此使用 Docker archive 交接镜像。运行用户不需要 Git、Rust、Node、GCC 或项目源码。

## 1. 管理员一次性准备运行用户

以下命令中的 `rkserve` 是示例专用用户名；如果已有专用用户，请替换为实际名称：

```bash
sudo usermod -aG video,render rkserve
sudo loginctl enable-linger rkserve
```

确认该用户在 `/etc/subuid` 和 `/etc/subgid` 中各有一段不与其他用户重叠的常规映射。新建用户时，发行版通常会自动分配；如果没有，应由管理员按本机现有规划分配，不要照抄其他用户的范围。

```bash
grep '^rkserve:' /etc/subuid /etc/subgid
```

随后让运行用户注销并重新登录。以运行用户执行以下预检：

```bash
id -nG
podman info --format 'rootless={{.Host.Security.Rootless}} cgroups={{.Host.CgroupsVersion}} runtime={{.Host.OCIRuntime.Name}}'
test -r /dev/dri/renderD129
test -w /dev/dri/renderD129
test -r /dev/rga
test -w /dev/rga
```

预期包含 `video`、`render`，并显示 `rootless=true`、`cgroups=v2`、`runtime=crun`。`GroupAdd=keep-groups` 依赖 `crun`。

## 2. 由编译用户导出镜像

在 RKServe 仓库中，以刚才完成镜像构建的普通用户执行：

```bash
transfer_dir=/var/tmp/rkserve-transfer
mkdir -p "$transfer_dir"
chmod 0755 "$transfer_dir"

./server/deploy/export-container-image.sh \
  localhost/rkserve:latest \
  "$transfer_dir/rkserve-image.tar"

install -m 0644 server/deploy/quadlet/rkserve.container "$transfer_dir/rkserve.container"
install -m 0644 server/deploy/Caddyfile.example "$transfer_dir/Caddyfile.example"
install -m 0755 plugins/install.sh "$transfer_dir/install-plugins.sh"
```

脚本拒绝覆盖已有 archive，并生成同名 `.sha256` 校验文件。若目录已经存在旧文件，请先人工确认版本并换一个新目录名，不要盲目删除。

## 3. 由运行用户导入镜像

切换到专用运行用户：

```bash
cd /var/tmp/rkserve-transfer
sha256sum --check rkserve-image.tar.sha256
podman load --input rkserve-image.tar
podman image inspect localhost/rkserve:latest \
  --format 'id={{.Id}} size={{.Size}} created={{.Created}}'
```

校验必须显示 `OK`。不同 Rootless 用户不能直接看到编译用户的镜像，`podman load` 后镜像才属于运行用户。

## 4. 创建 API Key secret

仍以运行用户执行。随机密钥只在终端显示一次，不写入仓库、Quadlet、镜像或环境变量持久化配置：

```bash
RKSERVE_KEY="$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')"
printf '{"keys":[{"name":"console","key":"%s","scopes":["read","infer","control","audit"]}]}\n' \
  "$RKSERVE_KEY" |
  podman secret create rkserve-api-keys -
printf '请立即保存到密码管理器，RKServe API Key：%s\n' "$RKSERVE_KEY"
unset RKSERVE_KEY
```

确认 secret 存在：

```bash
podman secret inspect rkserve-api-keys --format '{{.Spec.Name}}'
```

不要反复创建同名 secret。需要轮换时，应先准备新 secret、更新 Quadlet 引用并完成重启验证，再删除旧 secret。

## 5. 安装并启动用户级 Quadlet

把需要的插件从插件 Release 安装到 Quadlet 挂载的目录（容器内只读）。更新插件时重新执行同一命令，再重启服务。

```bash
mkdir -p "$HOME/.config/containers/systemd"
mkdir -p "$HOME/podman/rkserve/state" "$HOME/podman/rkserve/plugins"
chmod 0700 "$HOME/podman" "$HOME/podman/rkserve" "$HOME/podman/rkserve/state"

/var/tmp/rkserve-transfer/install-plugins.sh --dest "$HOME/podman/rkserve/plugins" yolo26 sensevoice-asr

install -m 0644 \
  /var/tmp/rkserve-transfer/rkserve.container \
  "$HOME/.config/containers/systemd/rkserve.container"

systemctl --user daemon-reload
systemctl --user start rkserve.service
```

Quadlet 的 `[Install]` 配置启动目标，无需对生成服务执行 `systemctl enable`。Quadlet 以 `Pull=never` 使用已导入的本地镜像。容器内部以 UID 0 运行，但该 UID 0 映射到宿主专用普通用户，不是宿主 root。运行时使用只读根文件系统、无 capabilities、`no-new-privileges`、有限设备映射和独立持久化目录。容器使用独立网络命名空间，只把 `8080` 发布到宿主 `127.0.0.1`，因此不会直接暴露到局域网，也不能访问宿主其他仅监听 localhost 的服务。

## 6. 验证

```bash
systemctl --user status rkserve.service --no-pager
podman ps --filter name=rkserve
podman healthcheck run rkserve
curl --fail --show-error http://127.0.0.1:8080/health
journalctl --user -u rkserve.service -n 100 --no-pager
```

再使用之前保存的 API Key 验证鉴权：

```bash
read -rsp 'RKServe API Key: ' RKSERVE_KEY
echo
curl --fail --show-error \
  -H "Authorization: Bearer ${RKSERVE_KEY}" \
  http://127.0.0.1:8080/api/v1/auth/whoami
unset RKSERVE_KEY
```

最后在浏览器控制台输入同一个 API Key，启动一个插件并完成一次真实 NPU 推理。仅 `/health` 成功不代表 NPU 设备权限已经验证。

## 7. 接入宿主 Caddy

以管理员身份把交接目录中的 `Caddyfile.example` 合并到宿主 Caddy 配置，替换域名，然后 reload Caddy。Caddy 应反向代理：

```text
127.0.0.1:8080
```

RKServe 只绑定宿主回环地址，没有直接暴露到局域网。不要在 Caddy access log 中记录 `Authorization` 或 `X-API-Key` 请求头。

## 8. 更新与回滚

编译用户构建新镜像后，使用新的交接目录和 archive。运行用户在导入前保留旧标签：

```bash
podman tag localhost/rkserve:latest localhost/rkserve:rollback
podman load --input /var/tmp/rkserve-transfer-new/rkserve-image.tar
systemctl --user restart rkserve.service
```

验证失败时回滚：

```bash
podman tag localhost/rkserve:rollback localhost/rkserve:latest
systemctl --user restart rkserve.service
```

确认新版本稳定后，再由 archive 的所有者删除 `/var/tmp` 中的交接文件。不要使用 `sudo podman` 管理这个服务；rootful Podman 看不到运行用户的容器、secret 和镜像。
