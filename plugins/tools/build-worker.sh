# Shared worker build steps, sourced by plugins/<id>/build.sh after it sets:
#   crate  Cargo package name of the worker
#   deps   dependency groups to fetch from plugins/deps.lock
#   libs   runtime libraries to bundle, as <path under .deps>[:<installed name>]
# The calling script takes one argument, the output directory, which receives
# bin/worker and lib/. Workers link aarch64 libraries and must be built on aarch64.
# shellcheck shell=bash disable=SC2154

plugin_dir="$(cd "$(dirname "${BASH_SOURCE[1]}")" && pwd)"
plugins_root="$(dirname "$plugin_dir")"
out="${1:?usage: $(basename "${BASH_SOURCE[1]}") <output-dir>}"
export RKSERVE_PLUGIN_DEPS="${RKSERVE_PLUGIN_DEPS:-${plugins_root}/.deps}"

if [[ "$(uname -m)" != "aarch64" ]]; then
  echo "Error: plugin workers must be built on an aarch64 host." >&2
  exit 1
fi

"${plugins_root}/tools/fetch-deps.sh" "${deps[@]}"
cargo build --locked --release --manifest-path "${plugins_root}/Cargo.toml" --package "$crate"

target_dir="${CARGO_TARGET_DIR:-${plugins_root}/target}"
install -Dm755 "${target_dir}/release/${crate}" "${out}/bin/worker"
strip --strip-unneeded "${out}/bin/worker"
for lib in "${libs[@]}"; do
  source_path="${lib%%:*}"
  name="${lib#*:}"
  [[ "$name" == "$lib" ]] && name="$(basename "$source_path")"
  install -Dm644 "${RKSERVE_PLUGIN_DEPS}/${source_path}" "${out}/lib/${name}"
done

mkdir -p "${out}/licenses"
repo_root="$(dirname "$plugins_root")"
{
  echo "Worker ${crate}: $(rustc --version), $(ldd --version | head -n 1)"
  echo "Source-Revision: $(git -C "$repo_root" rev-parse HEAD 2>/dev/null || echo unknown)"
  if git -C "$repo_root" status --porcelain --untracked-files=no 2>/dev/null | grep -q .; then
    echo "Source-Dirty: yes"
  else
    echo "Source-Dirty: no"
  fi
  echo "Downloaded build and runtime files (plugins/deps.lock):"
  for group in "${deps[@]}"; do
    grep "^${group}/" "${plugins_root}/deps.lock"
  done
} > "${out}/licenses/BUILD-DEPS.txt"
