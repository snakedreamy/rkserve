# SenseVoice ASR

English | [简体中文](README.zh-CN.md)

A multilingual SenseVoice-small speech-understanding plugin for RK3576. Its FP16 RKNN encoder produces text, language, emotion and sound-event results. Rust/C++ runtime; no Python, external model directories or other project checkouts.

## Inference pipeline

- WAV parsing, mixing and high-quality 16 kHz resampling: bundled C++.
- 80-dimensional Kaldi FBank, LFR `m=7,n=6`, CMVN and prompt embeddings: C++.
- SenseVoice-small encoder: fixed 344-frame RK3576 FP16 RKNN, using the leased NPU cores.
- CTC decoding, tag parsing, long-audio segmentation and text merging: C++.

The fixed encoder holds about 20 seconds of audio, but RK3576 FP16 can be numerically unstable for longer Chinese inputs. To prioritize accuracy, the plugin splits recordings at low-energy points into segments no longer than 10 seconds and returns timestamped results. The extra inference work is still substantially faster than real time.

## Recorded hardware validation

- RK3576 `core1`: about 0.65 seconds of NPU inference for a 5.55-second Chinese sample, with text substantially matching the FP32 ONNX baseline.
- A 33.28-second recording: six segments, about 3.64 seconds total NPU inference, retaining all speech.
- 44.1 kHz stereo 24-bit PCM input: English recognition verified after resampling.
- `core0_1` can reduce latency, but FP16 trailing punctuation may vary; the default is one core for stability.

Short segments are padded to 10 seconds of silence before feature extraction to avoid FP16 sensitivity to feature length. Returned duration and timestamps still use the original input, not the padded length. These are historical measurements, not new benchmarks from the localization change.

## Build

```bash
./build.sh
```

The model is SenseVoice-small, exported through `lovemefan/SenseVoice-onnx` and converted with RKNN Toolkit2 2.3.2 to RK3576 FP16. Its name and attribution are retained under the FunASR Model Open Source License Agreement.

## Use

Start SenseVoice on the NPU scheduling page, then upload WAV on the plugin run page. Automatic language detection covers more languages; explicitly select Chinese, English, Cantonese, Japanese or Korean when known. The console displays merged text, language, emotion, events and segment timelines, with copy-text and raw JSON download controls. Console language does not change the selected recognition language.
