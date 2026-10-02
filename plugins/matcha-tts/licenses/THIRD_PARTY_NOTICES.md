# Third-party components and model provenance

> The old artifact hashes below are historical provenance evidence, not results of the new build. New packages use their generated BUILD-INFO.txt, BUILD-DEPS.txt and MODEL-SOURCES.txt. Conversion reproducibility and redistribution authorization still require verification.

English | [简体中文](THIRD_PARTY_NOTICES.zh-CN.md)

This plugin does not require external project directories at runtime. It includes the following models, libraries, text assets and license files.

## Matcha Chinese/English model and text assets

- Source: `https://modelscope.cn/models/dengcunqin/matcha_tts_zh_en_20251010`
- Upstream model archive SHA256: `271b804af570400d3bcdcb53bf6e53cc9f75180ee763b9f13eb5eaf2b0d086ef`
- Original `model-steps-3.onnx` SHA256: `524286bf6cf11be74329ae1c682ac69e34d6860c2ea9fd1290319d561540b16a`
- `lexicon.txt` SHA256: `599efdcdaff4df2a123ce988c2cb90abcd59b919ef609ce36eb587d68b7ca2c0`
- `tokens.txt` SHA256: `77fee8e5e5dd96b3547119e6159292c648e99e065c54c97777722e3ce710b9a4`
- License: Apache License 2.0; see `Model-Apache-2.0`

`matcha-s64-fp16.rknn` was converted with RKNN-Toolkit2 for RK3576 from the original Matcha ONNX, fixed to 80 input positions and at most 599 output frames. Converted SHA256: `28d74f33fa8c8e35092af4f53518096d48f59a18111c2c7672dff8701a5befec`.

`matcha-duration.onnx` is an FP32 duration-prediction subgraph extracted from the same model to calibrate RKNN FP16 duration drift. SHA256: `b5571a905d8fe75de28e685cc4c952dfca727b49c247a1d94f4bc3d911c537b6`.

The original Matcha-TTS implementation uses the MIT License; see `Matcha-TTS-MIT`.

## Vocos

- Original model: `https://github.com/k2-fsa/sherpa-onnx/releases/download/vocoder-models/vocos-16khz-univ.onnx`
- Original ONNX SHA256: `b599142a1fb8ff03de3e84ac35ff537c619e56f4267a6fe894851a42844acf9e`
- RK3576 FP16 RKNN SHA256: `3aee1369a548ffde209bfc46ba36f06d60afac4c741e264ced3f61a7bdf73442`
- The original Vocos implementation uses the MIT License; see `Vocos-MIT`

## eSpeak NG

- Source commit: `https://github.com/csukuangfj/espeak-ng/commit/ed530aa113046142eb5115cf2fc9157854d0ffe1`
- Bundled executable SHA256: `1beaa4b0bd8441761fc6db9849720d8ace802c450abf32ebb72517761a5b7e07`
- Complete corresponding source archive: `espeak-ng-source-ed530aa.zip`
- Source archive SHA256: `e4e262cbe34f7fe21f91f1ba3397f2728e1f30eafbae7853f2b753a9ed13f0dd`
- License: GNU General Public License v3; see `eSpeak-NG-GPL-3.0`

The retained `espeak-ng-data` contains only dictionaries, phoneme tables and language settings needed for `en-us`, built from the commit above.

## ONNX Runtime

- Version: 1.26.0, Linux aarch64 CPU build
- Source: `https://github.com/microsoft/onnxruntime/tree/v1.26.0`
- `libonnxruntime.so.1` SHA256: `8cac3d595f13c6c87c7d9b0590083f299bdd654def63960a58c8407b8b112328`
- License: MIT; see `ONNX-Runtime-MIT`

## RKNN Runtime

`librknnrt.so` comes from the existing RK3576 RKNN Runtime package. SHA256: `d31fc19c85b85f6091b2bd0f6af9d962d5264a4e410bfb536402ec92bac738e8`. Use and redistribution are subject to the applicable Rockchip software license.

## Reference implementations

The text frontend and inference flow reference sherpa-onnx (Apache License 2.0) and `rkvoice-stream`. The plugin was reimplemented in Rust/C++ for the RKServe protocol and does not import those projects at runtime.
