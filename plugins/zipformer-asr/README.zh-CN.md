# Zipformer ASR

[English](README.md) | 简体中文

面向 RK3576 的中英文流式 Zipformer 语音识别插件。当前 RKServe 公共接口以异步文件任务方式调用；插件内部按 0.96 秒步长增量执行模型，为后续实时会话接口保留状态结构。

## 推理链路

- WAV 解析、混音与 16 kHz 重采样：插件内置 C++。
- 80 维 Kaldi FBank：内置编译 `kaldi-native-fbank` v1.20.3 源码。
- Encoder、Decoder、Joiner：三个 FP16 RKNN 模型，全部运行在租约指定的 NPU 核心。
- 解码：C++ 实现的 Transducer modified beam search，返回 UTF-8 纯文本。默认搜索宽度为 4，在当前模型仍明显快于实时的前提下优先保证识别效果；可按单次任务切换为 1、2 或 8。

模型来自 Rockchip `rknn_model_zoo/examples/zipformer` 所引用的 `k2fsa-zipformer-bilingual-zh-en-t`，并使用 RKNN Toolkit2 2.3.2 转换为 RK3576 FP16。

## 构建

```bash
./build.sh
```

插件目录包含运行所需的模型、词表、RKNN Runtime、FBank 源码和头文件，不依赖项目外目录或 Python。

## 使用

先在 NPU 调度页启动“Zipformer 中英文语音识别”，再进入插件运行页上传 WAV。默认参数适合常规使用；只有对延迟极度敏感时才建议把“解码搜索宽度”降为 `1`。

## 输入限制

输入必须为 RIFF/WAVE，支持 PCM 8/16/24/32-bit 与 IEEE Float32。多声道会平均混为单声道，非 16 kHz 输入使用 64-tap 加窗 sinc 重采样，以降低降采样混叠对识别率的影响。
