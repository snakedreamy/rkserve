# Matcha 中英文 TTS 插件

[English](README.md) | 简体中文

该插件为 RKServe 提供低延迟的中英文语音合成，不依赖 Python 或其他项目目录。

- Matcha 主声学网络：RK3576 FP16，NPU 执行。
- Vocos 16 kHz 声码器：RK3576 FP16，NPU 执行。
- 时长预测校准：精简 FP32 ONNX 子图在 CPU 执行，用于规避 RKNN FP16 时长漂移导致的停顿和重复；典型单段约 60–100 ms。
- 中文词典匹配、英文 eSpeak NG 音素转换、长文本分段、ISTFT 和 WAV 编码：原生 C/C++ 执行。
- 单段最多 64 个音素，超过限制时自动分段，不截断输入。
- 输出为 16 kHz 单声道 16-bit PCM WAV。

运行时需要的模型、词典、英文音素数据、eSpeak NG、ONNX Runtime 和 RKNN Runtime 都位于插件目录内，不读取 `rkvoice-stream` 或其他项目目录，也不启动 Python。

```bash
curl -X POST \
  'http://127.0.0.1:8080/api/v1/plugins/matcha-tts/jobs/audio.speech?speed=1.0' \
  -H "Authorization: Bearer ${RKSERVE_API_KEY}" \
  -H 'Content-Type: text/plain; charset=utf-8' \
  --data-binary '你好，welcome to RKServe。'
```

模型转换说明、文件校验值及第三方许可见 `licenses/THIRD_PARTY_NOTICES.md`。
