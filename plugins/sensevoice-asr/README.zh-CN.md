# SenseVoice ASR

[English](README.md) | 简体中文

面向 RK3576 的 SenseVoice-small 多语言语音理解插件。插件使用 FP16 RKNN Encoder，同时输出转写文本、语言、情绪和声音事件；运行时为 Rust + C++，不依赖 Python、外部模型目录或其他项目。

## 推理链路

- WAV 解析、混音与 16 kHz 高质量重采样：插件内置 C++。
- 80 维 Kaldi FBank、LFR `m=7,n=6`、CMVN 与 Prompt Embedding：C++。
- SenseVoice-small Encoder：固定 344 帧的 RK3576 FP16 RKNN，运行在租约指定的 NPU 核心。
- CTC 解码、特殊标签解析、长音频分段与文本合并：C++。

固定 Encoder 容量大约覆盖 20 秒语音，但 RK3576 FP16 在部分较长中文输入上存在数值不稳定。插件以准确率优先，将长录音按不超过 10 秒的低能量位置切分并返回带起止时间的分段结果；额外推理开销仍明显快于实时。

## 真机验证

- RK3576 `core1`：5.55 秒中文样例的 NPU 推理约 0.65 秒，文本与 FP32 ONNX 基准主体一致。
- 33.28 秒长音频：自动切为 6 段，NPU 推理合计约 3.64 秒，全部语音内容均被保留。
- 已验证 44.1 kHz、双声道、24-bit PCM 输入，高质量重采样后可正确识别英文。
- `core0_1` 可进一步降低延迟，但 FP16 尾部标点可能变化，因此默认使用单核以优先保证结果稳定性。

短段会在特征提取前补齐为固定 10 秒静音长度。这不是输出时长填充，而是为规避 RK3576 FP16 对输入特征长度的数值敏感；返回的音频时长与分段时间仍使用原始输入。

## 构建

```bash
./build.sh
```

模型来自 `SenseVoice-small`，通过 `lovemefan/SenseVoice-onnx` 的 Encoder 导出，并使用 RKNN Toolkit2 2.3.2 转换为 RK3576 FP16。模型名称和来源按 FunASR Model Open Source License Agreement 保留。

## 使用

先在 NPU 调度页启动“SenseVoice 多语言语音理解”，再进入插件运行页上传 WAV。语言保持“自动检测”可覆盖更多语种；已知语言为中、英、粤、日、韩时可明确指定。网页会显示合并文本、语言、情绪、声音事件以及长音频逐段时间轴，并可复制文本或下载原始 JSON。
