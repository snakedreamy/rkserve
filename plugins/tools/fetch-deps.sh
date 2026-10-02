#!/usr/bin/env bash
# Download and verify the third-party files listed in plugins/deps.lock.
# Usage: plugins/tools/fetch-deps.sh <group>...   (for example: rknpu2 librga)
# Files are stored under plugins/.deps (or $RKSERVE_PLUGIN_DEPS); .tgz archives
# are also extracted into their group directory.
set -euo pipefail

plugins_root="$(cd "$(dirname "$0")/.." && pwd)"
lock="${plugins_root}/deps.lock"
deps_root="${RKSERVE_PLUGIN_DEPS:-${plugins_root}/.deps}"

if (($# == 0)); then
  echo "Usage: $0 <group>..." >&2
  exit 2
fi
for tool in curl sha256sum tar; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Error: ${tool} was not found." >&2
    exit 1
  fi
done

for group in "$@"; do
  found=0
  while read -r path sha256 url; do
    [[ -z "$path" || "$path" == \#* || "$path" != "${group}/"* ]] && continue
    found=1
    target="${deps_root}/${path}"
    if [[ ! -f "$target" ]] || ! echo "${sha256}  ${target}" | sha256sum --quiet --check - >/dev/null 2>&1; then
      mkdir -p "$(dirname "$target")"
      curl --fail --silent --show-error --location --retry 3 --output "${target}.part" "$url"
      if ! echo "${sha256}  ${target}.part" | sha256sum --quiet --check - >/dev/null 2>&1; then
        rm -f -- "${target}.part"
        echo "Error: checksum mismatch for ${path} from ${url}" >&2
        exit 1
      fi
      mv -f -- "${target}.part" "$target"
      echo "fetched ${path}"
    fi
    if [[ "$path" == *.tgz ]]; then
      tar --extract --gzip --no-same-owner --strip-components=1 \
        --directory "${deps_root}/${group}" --file "$target"
    fi
  done < "$lock"
  if ((found == 0)); then
    echo "Error: ${lock} has no entries for ${group}." >&2
    exit 1
  fi
done
