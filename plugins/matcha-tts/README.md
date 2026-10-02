# Matcha Chinese/English TTS plugin

English | [简体中文](README.zh-CN.md)

Low-latency Chinese/English speech synthesis for RKServe, without Python or external project directories.

- Matcha acoustic network: RK3576 FP16 on the NPU.
- Vocos 16 kHz vocoder: RK3576 FP16 on the NPU.
- Duration calibration: a compact FP32 ONNX subgraph on the CPU avoids pauses/repetition caused by RKNN FP16 duration drift; typically 60–100 ms per segment.
- Chinese lexicon matching, English eSpeak NG phonemization, text segmentation, ISTFT and WAV encoding: native C/C++.
- At most 64 phonemes per segment; longer input is split, never truncated.
- Output: 16 kHz mono 16-bit PCM WAV.

Models, lexicons, English phoneme data, eSpeak NG, ONNX Runtime and RKNN Runtime reside in the plugin directory after fetching assets. No `rkvoice-stream` checkout or Python process is required.

After starting the plugin, submit mixed-language text (the text is inference input, not UI localization):

```bash
curl -X POST \
  'http://127.0.0.1:8080/api/v1/plugins/matcha-tts/jobs/audio.speech?speed=1.0' \
  -H "Authorization: Bearer ${RKSERVE_API_KEY}" \
  -H 'Content-Type: text/plain; charset=utf-8' \
  --data-binary '你好，welcome to RKServe。'
```

Model conversion details, checksums and licenses are in [third-party notices](licenses/THIRD_PARTY_NOTICES.md).
