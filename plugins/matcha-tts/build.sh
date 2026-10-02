#!/usr/bin/env bash
# Build the Matcha TTS worker and eSpeak NG, and stage bin/, lib/, the en-us
# eSpeak NG data and the eSpeak NG source archive into the given directory.
set -euo pipefail

crate=rkserve-plugin-matcha-tts
deps=(rknpu2 onnxruntime)
libs=(rknpu2/lib/librknnrt.so onnxruntime/lib/libonnxruntime.so.1.26.0:libonnxruntime.so.1)
# shellcheck source-path=SCRIPTDIR source=../tools/build-worker.sh
source "$(dirname "$0")/../tools/build-worker.sh"

# eSpeak NG is GPL-3.0; the package ships the exact source it was built from.
espeak_commit=ed530aa113046142eb5115cf2fc9157854d0ffe1
espeak_src="${RKSERVE_PLUGIN_DEPS}/espeak-ng/src"
espeak_build="${RKSERVE_PLUGIN_DEPS}/espeak-ng/build"
if [[ "$(git -C "$espeak_src" rev-parse HEAD 2>/dev/null)" != "$espeak_commit" ]]; then
  rm -rf -- "$espeak_src" "$espeak_build"
  git init --quiet "$espeak_src"
  git -C "$espeak_src" fetch --quiet --depth 1 https://github.com/csukuangfj/espeak-ng.git "$espeak_commit"
  git -C "$espeak_src" checkout --quiet FETCH_HEAD
fi
cmake -S "$espeak_src" -B "$espeak_build" -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF \
  -DESPEAK_BUILD_MANPAGES=OFF -DESPEAK_COMPAT=OFF -DUSE_ASYNC=OFF -DUSE_KLATT=OFF \
  -DUSE_LIBPCAUDIO=OFF -DUSE_LIBSONIC=OFF -DUSE_MBROLA=OFF -DUSE_SPEECHPLAYER=OFF
cmake --build "$espeak_build" --parallel "$(nproc)"

install -Dm755 "${espeak_build}/src/espeak-ng-bin" "${out}/bin/espeak-ng"
strip --strip-unneeded "${out}/bin/espeak-ng"
# Only the data needed for en-us phonemization is bundled.
for file in en_dict intonations lang/gmw/en lang/gmw/en-US phondata phondata-manifest phonindex phontab; do
  install -Dm644 "${espeak_build}/espeak-ng-data/${file}" "${out}/assets/text/espeak-ng-data/${file}"
done
mkdir -p "${out}/licenses"
git -C "$espeak_src" archive --format=tar.gz --prefix="espeak-ng-${espeak_commit}/" \
  --output "${out}/licenses/espeak-ng-source-${espeak_commit:0:7}.tar.gz" HEAD
