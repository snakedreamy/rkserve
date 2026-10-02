# Five YOLO26 FP16 model conversions

English | [简体中文](README.zh-CN.md)

## Sources and transformations

Pin official yolo26n/s/m/l/x.pt assets from `ultralytics/assets` release `v8.4.0` using the GitHub release digests in `inputs.json`. The n variant was downloaded and rehashed; the other four digests were obtained from the GitHub API without a fresh local download.

Pin Ultralytics 8.4.103 and additionally retain a complete git archive of its v8.4.103 commit `bdc65f629a533f494da2a73cb8e201ea11943b54`, with pre-verified SHA `dd8fd8b61cae9d046aad3f34f1d0ac48ec8d2b249ee20150c0764f401ec58abf`. This conversion script follows the AGPL-3.0 Exporter flow under the same license. Retain complete upstream source, PT inputs, conversion scripts, and licenses in subsequent source packages.

Model settings: format=rknn, name=rk3576, imgsz640, batch1, quantize16, end2end=False, dynamic=False, nms=False, device=cpu, opset19. Preserve the `format=rknn` context rather than taking the separate ONNX-half export path. Override the export_rknn stage to call onnxslim explicitly: upstream catches simplifier exceptions and merely warns; this pipeline must fail instead.

RKNN: mean `[0,0,0]`, std `[255,255,255]`, optimization level 3, float16, no quantization, rknn_batch_size=1. Input `[1,3,640,640]`, output `[1,84,8400]`. Write `yolo26{n,s,m,l,x}_rknn_fp16_640` directories containing `yolo26{size}-rk3576.rknn` and metadata.yaml, not upstream's default `_rknn_model` layout.

Metadata description contains only a public COCO reference. The date uses `SOURCE_DATE_EPOCH` (default 0), including ONNX metadata; no model author's local training path is retained there. The worker recursively discovers RKNN files with sibling metadata, so this layout matches its contract.

## Verified evidence and gaps

Historical metadata identifies 8.4.103; the historical RKNN compiler is also x86 2.3.2. No original conversion logs or intermediate ONNX were retained, so byte-identical binary reproduction is unproven. Date normalization intentionally changes metadata hashes. Upstream only constrains toolkit>=2.3.2 and ONNX<1.19; this pipeline pins more strictly.

**A PT checkpoint is published trained weights, not complete training source.** Retaining official training implementation does not establish availability of training data, exact training recipes, or every preferred source form; AGPL corresponding-source scope still requires legal review. Investigate git-archive hash mismatches rather than removing the check.

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
