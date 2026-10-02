# SenseVoice-small conversion

English | [简体中文](README.zh-CN.md)

## Sources and transformations

Pin HF repository `lovemefan/SenseVoice-onnx` to commit `8a5ee5b014950890a07246bc590a4f77b3ef67a4` for the encoder, am.mvn, embedding.npy, and SentencePiece BPE model. Full URLs/hashes are in `inputs.json`; the original encoder SHA is `b1145eb7e9d2dbb9005fc99ed0313efebfadffea4aee1f4fa68b69a359126227`.

`vendor/sv_fix_shape.py` fixes speech to `[1,344,560]`, removes speech_lengths from graph inputs, inserts an INT64 `[344]` initializer, and runs shape inference. It comes from rkvoice-stream commit `2266a05bdda637c13ab05f32ccdd6220621b0ec4`, original SHA `c1f8cf7a6539d692fcf289a7d8ac605aa6f909c9e6f968fcf39af998a3c1a495`. Only a provenance header was added; the Apache-2.0 license is retained with the output.

RKNN settings: rk3576, optimization level 3, float16, default mean/std 0/1, no quantization or dynamic_input. Explicitly load a single speech input `[1,344,560]`, producing `[1,344,25055]`. Generate tokens as `piece id\n` for all 25055 `id_to_piece(i)` entries; copy am.mvn/embedding unchanged.

Outputs are `sense-voice-encoder.rk3576.fp16.rknn` and the three text files. The fixed ONNX (SHA `e81639fff8e672475bf550f1f681c0da10361cd7c5f3ddee1d34956788fb428d`) and tokens were reproduced byte-for-byte in the retained environment; the script enforces those hashes.

## Verified evidence and gaps

The historical RKNN matches the retained workspace, but conversion commands/logs are missing; optimization level 3 is inferred from the reference converter. lovemefan is a **third-party ONNX export**, not a complete Alibaba checkpoint-to-ONNX source chain. Retain both the FunASR model license and the conversion repository's MIT declaration. Do not relicense SenseVoice-small weights as MIT; review obligations under the FunASR Model Open Source License Agreement v1.1 before release.

## Invocation and output contract

On a GitHub-hosted `ubuntu-24.04` x86_64 runner, select **Python 3.11** with `actions/setup-python`, then explicitly run this directory's `convert.sh <absolute-output-directory>`. The output directory must be absent or empty; existing artifacts are never overwritten. `CONVERSION_PYTHON` may select a Python 3.11 executable. This implementation has not run downloads, conversion, compilation, or dependency installation.

The entrypoint creates a separate temporary venv outside the output directory. It installs only the individually pinned wheels in `plugins/tools/conversion-requirements.txt` and this directory's `requirements.txt`. `--no-deps` prohibits unpinned transitive dependencies; `pip check` must pass. Torch uses official CPU wheels. The RKNN x86 cp311 wheel SHA is fixed to `a05a8fd7515705ebdbb06a965c80b0090af61f7e716f1ed326a3aa5e313f23fb`. Other dependencies have fixed distribution versions, and their downloaded wheels and actual SHA values are retained; a pre-approved hash lock for every dependency wheel has not yet been established.

The output root contains **only `assets/` and `licenses/`**. It never supplies or overwrites a plugin manifest, `bin/`, or `lib/`.

- `assets/`: exactly the model/data files in `outputs.json`, excluding runtime libraries and workers.
- `licenses/MODEL-SOURCES.txt`: provenance, transformations, licensing information, and a copy of this document.
- `licenses/model-sources/inputs/` and `INPUTS.json`: original ONNX/PT files, data, fixed download URLs, SHA256 values, and license information. These are not removed after success.
- `licenses/model-sources/conversion/`: the actual conversion scripts, graph-processing source, licenses, input/output manifests, and environment pins used by this run.
- `licenses/model-sources/environment/`: complete wheels, pinned requirements, pip freeze, Python/system information, and logs.
- `licenses/model-sources/intermediates/`: generated ONNX graphs and hybrid-quantization intermediate files.
- `licenses/model-sources/SHA256SUMS`: hashes of the retained inputs, sources, and dependencies, computed after logging finishes.
- `licenses/OUTPUTS.sha256`: hashes of assets and licenses, excluding the checksum file itself and the temporary failure marker. Failures retain `licenses/CONVERSION-INCOMPLETE`; downstream validation may proceed only after a successful process exit and removal of that marker.

**Source packages must retain the entire `licenses/model-sources/` tree, not just logs or models.** Temporary work lives under `/tmp`; it is removed on success and its path is printed on failure for diagnosis. The tool never calls `init_runtime`, accesses an NPU, or installs system packages. The runner needs Git, Bash, Python 3.11/venv, and any system shared libraries required by OpenCV. Peak memory/disk usage has not been validated on GitHub runners.

A pinned environment is not proof of byte-identical RKNN reproduction. Historical audio/Pose models used the arm64 compiler `2.3.2 (@2025-04-03T08:26:16)`; this pipeline uses the x86_64 compiler `2.3.2 (e045de294f@2025-04-07T19:48:25)`. Never substitute historical checksums for new artifacts; separate quality and NPU acceptance tests remain necessary.

## Licensing and release gates

This pipeline starts from **published ONNX/PT artifacts**, not complete training data, training code, or every preferred form for modification. Successful conversion is not legal proof that GPL/AGPL corresponding-source obligations are satisfied. Production use and redistribution of RKNN SDK components require separate confirmation. Including license texts does not establish permission to redistribute models, calibration images, dependency wheels, or runtime libraries.
