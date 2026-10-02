# Release process

English | [简体中文](releasing.zh-CN.md)

The application and plugins are built and published separately from the same repository.

## Application

`server/Containerfile` builds Core and the console only. It does not download plugin models or the RKNN SDK.

Push a `v*` tag (for example `v0.1.0`) to run the `Image` workflow, which publishes a `linux/arm64` image to GHCR as `ghcr.io/snakedreamy/rkserve`. Semver tags also update `latest`.

Manual dispatch of that workflow publishes an image tagged with the commit SHA; it does not update `latest`. After the first push, set the GHCR package visibility to public if the repository is public.

## Plugins

Push a `plugins-v*` tag (for example `plugins-v0.1.0`) to run the `Plugins` workflow:

1. Convert models from pinned inputs on an x86_64 runner.
2. Build workers on an ARM64 runner, downloading third-party files listed in `plugins/deps.lock`.
3. Package each plugin as `rkserve-plugin-<id>.tar.gz`.
4. Create a GitHub Release with those archives and `SHA256SUMS`.

Users install a published release with `plugins/install.sh`. Manual dispatch of the workflow builds the same packages without creating a release.

Do not overwrite an existing release tag.

## Checks

```bash
python3 -m unittest discover -s plugins/tests -v
shellcheck -x server/deploy/*.sh plugins/install.sh plugins/tools/*.sh plugins/*/build.sh plugins/*/convert/*.sh
(cd server && cargo test --locked -p rkserve-core -p rkserve-protocol)
(cd server/frontend && pnpm test)
```

Model conversion and image builds run on GitHub. After a plugin release, install the packages on an RK3576 and confirm workers load, inference runs, and the console and API still authenticate.
