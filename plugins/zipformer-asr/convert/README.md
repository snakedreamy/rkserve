# Bilingual streaming Zipformer conversion

English | [简体中文](README.zh-CN.md)

## Sources and transformations

Use Rockchip model-zoo's three published `*-epoch-99-avg-1.onnx` files at the fixed Filez delivery URLs in `inputs.json`. Retain the original convert.py and vocabulary from model-zoo commit `bad6c7334531becaf90a561988519b7bec34d0ab`. The models and reference code are labeled Apache-2.0.

For each static ONNX, run RKNN config, load_onnx, build, and export with rk3576, optimization level 3, float16, no quantization, default mean/std 0/1, and no dynamic_input. This corresponds to reference invocation `convert.py <onnx> rk3576 fp <output>`, with explicit return-code validation added.

The encoder has 36 inputs and 36 outputs; its first input is `[1,103,80]`. Decoder: `[1,2]` to `[1,512]`; joiner: two `[1,512]` inputs to `[1,6254]`. Outputs are `encoder.rknn`, `decoder.rknn`, `joiner.rknn`, and an unchanged vocab.txt.

## Verified evidence and gaps

These ONNX files **differ in hash/size** from the same-named files in k2's HF repository `csukuangfj/k2fsa-zipformer-bilingual-zh-en-t`. The exact k2-to-Rockchip static-shape/optimization transformation was not found. This script starts from Rockchip's artifacts and must not be presented as filling that gap. Historical RKNN files match the local workspace, but the original commands/logs were not retained; optimization level 3 is the pinned toolkit default, not a setting recovered from historical logs.

## Invocation and output contract

On a GitHub-hosted `ubuntu-24.04` x86_64 runner, select **Python 3.11** with `actions/setup-python`, then explicitly run this directory's `convert.sh <absolute-output-directory>`. The output directory must be absent or empty; existing artifacts are never overwritten. `CONVERSION_PYTHON` may select a Python 3.11 executable.

The entrypoint creates a separate temporary venv outside the output directory. It installs only the individually pinned wheels in `plugins/tools/conversion-requirements.txt` and this directory's `requirements.txt`. `--no-deps` prohibits unpinned transitive dependencies; `pip check` must pass. Torch uses official CPU wheels. The RKNN x86 cp311 wheel SHA is fixed to `a05a8fd7515705ebdbb06a965c80b0090af61f7e716f1ed326a3aa5e313f23fb`. Other dependencies have pinned distribution versions; downloaded wheels and their SHA-256 values are retained in the conversion output.

The output root contains **only `assets/` and `licenses/`**. It never supplies or overwrites a plugin manifest, `bin/`, or `lib/`.

- `assets/`: exactly the model/data files in `outputs.json`, excluding runtime libraries and workers.
- `licenses/MODEL-SOURCES.txt`: provenance, transformations, licensing information, and a copy of this document.
- `licenses/model-sources/inputs/` and `INPUTS.json`: original ONNX/PT files, data, fixed download URLs, SHA256 values, and license information. These are not removed after success.
- `licenses/model-sources/conversion/`: the actual conversion scripts, graph-processing source, licenses, input/output manifests, and environment pins used by this run.
- `licenses/model-sources/environment/`: complete wheels, pinned requirements, pip freeze, Python/system information, and logs.
- `licenses/model-sources/intermediates/`: generated ONNX graphs and hybrid-quantization intermediate files.
- `licenses/model-sources/SHA256SUMS`: hashes of the retained inputs, sources, and dependencies, computed after logging finishes.
- `licenses/OUTPUTS.sha256`: hashes of assets and licenses, excluding the checksum file itself and the temporary failure marker. Failures retain `licenses/CONVERSION-INCOMPLETE`; downstream validation may proceed only after a successful process exit and removal of that marker.

**Source packages must retain the entire `licenses/model-sources/` tree, not just logs or models.** Temporary work lives under `/tmp`; it is removed on success and its path is printed on failure for diagnosis. The tool never calls `init_runtime`, accesses an NPU, or installs system packages. The runner needs Git, Bash, Python 3.11/venv, and any system shared libraries required by OpenCV.

A pinned environment is not proof of byte-identical RKNN reproduction. Historical audio/Pose models used the arm64 compiler `2.3.2 (@2025-04-03T08:26:16)`; this pipeline uses the x86_64 compiler `2.3.2 (e045de294f@2025-04-07T19:48:25)`. Never substitute historical checksums for new artifacts; separate quality and NPU acceptance tests remain necessary.
