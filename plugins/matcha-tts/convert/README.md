# Matcha Chinese/English TTS conversion

English | [简体中文](README.zh-CN.md)

## Sources and transformations

The model author is dengcunqin; the model page is `https://modelscope.cn/models/dengcunqin/matcha_tts_zh_en_20251010`. Use sherpa-onnx's `tts-models/matcha-icefall-zh-en.tar.bz2` (SHA `271b804af570400d3bcdcb53bf6e53cc9f75180ee763b9f13eb5eaf2b0d086ef`) and `vocoder-models/vocos-16khz-univ.onnx` (SHA `b599142a1fb8ff03de3e84ac35ff537c619e56f4267a6fe894851a42844acf9e`). Full URLs and hashes are in `inputs.json`.

1. Extract the original `model-steps-3.onnx` and expose the INT64 scalar `/ReduceMax_output_0` as a second output. Extract the FP32 duration graph from this augmented graph, with inputs `x,x_length,length_scale` and the length output.
2. Run `vendor/fix_matcha_rknn.py --seq-len 80 --x-len 64`: probe the original graph, fix inputs with onnxsim, replace Range with constants, replace Ceil with Neg/Floor/Neg, fix Slice ends to 599, create fixed `[1,80,600]` noise using `default_rng(42)`, and infer shapes. The script comes from rkvoice-stream commit `2266a05bdda637c13ab05f32ccdd6220621b0ec4`; original SHA `c714fc11f073a12e7102a0f1edf203c2e41ddc14e4c688cb44e27ee3b4cb76be`. Only a provenance header was added; the complete Apache-2.0 license accompanies the output.
3. Acoustic RKNN: rk3576, optimization level 0, float16, no quantization, default mean/std 0/1, no dynamic_input; outputs are mel `[1,80,599]` and actual length. Despite the output filename `matcha-s64-fp16.rknn`, the graph must be the length-output variant.
4. Vocos: rk3576, optimization level 3, float16, single_core_mode=False, no quantization; fixed mels `[1,80,600]`, three `[1,513,600]` outputs. Do not apply the RK3588-only Clip workaround.
5. Copy lexicon/tokens from the model archive. The worker build generates the eight en-us eSpeak data files alongside its executable; this conversion step does not copy old phondata over the new build. Also verify and retain the complete csukuangfj/espeak-ng source zip at commit `ed530aa113046142eb5115cf2fc9157854d0ffe1`, SHA `e4e262cbe34f7fe21f91f1ba3397f2728e1f30eafbae7853f2b753a9ed13f0dd`, for subsequent GPL source packaging.

Outputs are `matcha-duration.onnx`, `matcha-s64-fp16.rknn`, `vocos-16khz-600-fp16.rknn`, and text data. A separate plugin build must supply the aarch64 eSpeak executable, its matching data and runtime libraries; this entrypoint does not generate `bin/espeak-ng`.

## Verified evidence and gaps

The duration graph extracted from the augmented ONNX is hash-checked. The subsequent probe-first rewrite is validated by ONNX Runtime inside `fix_matcha_rknn.py` (output shape and cosine similarity); its bytes are retained as an intermediate, not treated as a pinned input. Historical RKNN commands/logs are missing: optimization levels and single-core configuration are inferred from reference scripts, not proven historical settings. Local compiled eSpeak data matches the eight archive files, but the complete build toolchain, original model training project, and Vocos weight-training provenance remain incomplete.

Model/text: Apache-2.0; Matcha/Vocos implementations: MIT; eSpeak executable/data: GPL-3.0. These licenses do not replace one another.

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
