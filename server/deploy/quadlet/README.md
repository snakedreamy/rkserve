# RKServe rootless Quadlet deployment

English | [简体中文](README.zh-CN.md)

This guide targets Podman 5.4.2+ with user-level Quadlet services. The runtime user needs no Git, Rust, Node, GCC or source checkout.

To use the published image, pull `ghcr.io/snakedreamy/rkserve:<version>`, set that name as `Image=` in the Quadlet unit, and remove `Pull=never`. Skip the export/import steps; the rest of this guide (runtime user, secret, plugin directory, Caddy) still applies.

The export/import steps below hand a locally built image from a build user to a separate runtime user. Their rootless Podman storage is isolated, so the image is transferred as a Docker archive.

## 1. One-time runtime-user setup by an administrator

`rkserve` is an example dedicated account. Substitute your existing service account if applicable:

```bash
sudo usermod -aG video,render rkserve
sudo loginctl enable-linger rkserve
```

Ensure the user has non-overlapping subordinate UID/GID ranges in `/etc/subuid` and `/etc/subgid`. Distributions commonly allocate these when creating accounts; otherwise an administrator must assign ranges consistent with the host, not copy another account's ranges.

```bash
grep '^rkserve:' /etc/subuid /etc/subgid
```

Log the runtime user out and back in, then run these checks as that user:

```bash
id -nG
podman info --format 'rootless={{.Host.Security.Rootless}} cgroups={{.Host.CgroupsVersion}} runtime={{.Host.OCIRuntime.Name}}'
test -r /dev/dri/renderD129
test -w /dev/dri/renderD129
test -r /dev/rga
test -w /dev/rga
```

Expect `video` and `render` groups, `rootless=true`, `cgroups=v2` and `runtime=crun`. `GroupAdd=keep-groups` requires `crun`.

## 2. Export the image as the build user

From the RKServe repository, run as the regular user who built the image:

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

The script refuses to overwrite archives and generates a matching `.sha256` file. If old files exist, inspect their versions and choose a new directory rather than deleting blindly.

## 3. Import the image as the runtime user

Switch to the dedicated runtime account:

```bash
cd /var/tmp/rkserve-transfer
sha256sum --check rkserve-image.tar.sha256
podman load --input rkserve-image.tar
podman image inspect localhost/rkserve:latest \
  --format 'id={{.Id}} size={{.Size}} created={{.Created}}'
```

Checksum verification must report `OK`. Other rootless users cannot see the builder's images; `podman load` imports the image into the runtime user's storage.

## 4. Create an API key secret

Continue as the runtime user. Display the random key once and do not persist it in Git, Quadlet, images or environment configuration:

```bash
RKSERVE_KEY="$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')"
printf '{"keys":[{"name":"console","key":"%s","scopes":["read","infer","control","audit"]}]}\n' \
  "$RKSERVE_KEY" |
  podman secret create rkserve-api-keys -
printf 'Save this RKServe API key in a password manager now: %s\n' "$RKSERVE_KEY"
unset RKSERVE_KEY
```

Verify the secret exists:

```bash
podman secret inspect rkserve-api-keys --format '{{.Spec.Name}}'
```

Do not repeatedly create a secret with the same name. To rotate, prepare a new secret, update Quadlet, restart and verify, then remove the old secret.

## 5. Install and start the user-level Quadlet

Install the plugins you need from the plugin release into the directory mounted by the Quadlet (read-only in the container). Run the same command again to update a plugin, then restart the service.

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

Quadlet uses the imported image with `Pull=never`. Container UID 0 maps to the regular service account, not host root. The runtime uses a read-only root filesystem, no capabilities, `no-new-privileges`, limited device mappings and separate persistent storage. Its network namespace is isolated; port 8080 is published only on host `127.0.0.1`, not the LAN, and it cannot access other host localhost services. The unit's `[Install]` section configures startup; generated Quadlet services are not enabled with `systemctl enable`.

## 6. Verify

```bash
systemctl --user status rkserve.service --no-pager
podman ps --filter name=rkserve
podman healthcheck run rkserve
curl --fail --show-error http://127.0.0.1:8080/health
journalctl --user -u rkserve.service -n 100 --no-pager
```

Verify authentication using the saved key:

```bash
read -rsp 'RKServe API Key: ' RKSERVE_KEY
echo
curl --fail --show-error \
  -H "Authorization: Bearer ${RKSERVE_KEY}" \
  http://127.0.0.1:8080/api/v1/auth/whoami
unset RKSERVE_KEY
```

Finally, enter the same key in the console, start a plugin and run actual NPU inference. A successful `/health` check does not verify NPU device permissions.

## 7. Configure host Caddy

As an administrator, merge the handoff directory's `Caddyfile.example` into host Caddy configuration, replace the domain, and reload Caddy. Proxy to:

```text
127.0.0.1:8080
```

RKServe binds only to loopback, not directly to the LAN. Do not log `Authorization` or `X-API-Key` in Caddy access logs.

## 8. Update and roll back

After building a new image, use a new handoff directory/archive. The runtime user preserves the previous tag before importing:

```bash
podman tag localhost/rkserve:latest localhost/rkserve:rollback
podman load --input /var/tmp/rkserve-transfer-new/rkserve-image.tar
systemctl --user restart rkserve.service
```

Roll back if verification fails:

```bash
podman tag localhost/rkserve:rollback localhost/rkserve:latest
systemctl --user restart rkserve.service
```

Once the new version is stable, the archive owner can remove handoff files from `/var/tmp`. Do not manage this service with `sudo podman`; rootful Podman cannot see the runtime user's containers, secrets or images.
