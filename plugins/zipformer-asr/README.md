# Zipformer ASR

English | [简体中文](README.zh-CN.md)

Chinese/English streaming Zipformer recognition for RK3576. RKServe currently exposes asynchronous file jobs; internally the plugin advances the model in 0.96-second steps, retaining state structures suitable for future real-time sessions.

## Inference pipeline

- WAV parsing, mixing and 16 kHz resampling: bundled C++.
- 80-dimensional Kaldi FBank: compiled `kaldi-native-fbank` v1.20.3 sources.
- Encoder, decoder and joiner: three FP16 RKNN models on the leased NPU cores.
- Decoding: C++ Transducer modified beam search, returning UTF-8 plain text. Beam size defaults to 4 to favor accuracy while remaining faster than real time in recorded testing; requests may select 1, 2 or 8.

Models originate from `k2fsa-zipformer-bilingual-zh-en-t`, referenced by Rockchip `rknn_model_zoo/examples/zipformer`, and were converted with RKNN Toolkit2 2.3.2 to RK3576 FP16.

## Build

```bash
./build.sh
```

After fetching assets, the plugin directory contains the models, vocabulary, RKNN Runtime, FBank sources and headers. No external checkout or Python is required.

## Use

Start Zipformer on the NPU scheduling page, then upload WAV on its run page. Defaults suit normal use; reduce the decoding beam size to `1` only when latency is critical.

## Input limits

Input must be RIFF/WAVE with PCM 8/16/24/32-bit or IEEE Float32. Multichannel audio is averaged to mono; non-16 kHz audio uses 64-tap windowed-sinc resampling to reduce downsampling aliasing.
