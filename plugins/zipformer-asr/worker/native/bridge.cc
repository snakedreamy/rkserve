#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <limits>
#include <map>
#include <memory>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

#include "kaldi-native-fbank/csrc/online-feature.h"
#include "rknn_api.h"

namespace {

constexpr int kSampleRate = 16000;
constexpr int kMelBins = 80;
constexpr int kSegmentFrames = 103;
constexpr int kSegmentOffset = 96;
constexpr int kEncoderOutputFrames = 24;
constexpr int kDecoderDimension = 512;
constexpr int kJoinerVocabulary = 6254;
constexpr int kContextSize = 2;
constexpr int64_t kBlankId = 0;
constexpr int64_t kUnknownId = 2;

using Clock = std::chrono::steady_clock;

struct NativeTiming {
  uint64_t preprocess_us;
  uint64_t inference_us;
  uint64_t postprocess_us;
  uint64_t audio_duration_ms;
  uint64_t token_count;
};

struct AudioData {
  std::vector<float> samples;
  int sample_rate = 0;
};

struct ModelOutputs {
  std::vector<std::vector<float>> floats;
  std::vector<std::vector<int64_t>> integers;
};

uint64_t elapsed_us(Clock::time_point start, Clock::time_point end) {
  return static_cast<uint64_t>(
      std::chrono::duration_cast<std::chrono::microseconds>(end - start).count());
}

void write_error(char* output, size_t capacity, const std::string& message) {
  if (output != nullptr && capacity != 0) {
    std::snprintf(output, capacity, "%s", message.c_str());
  }
}

std::vector<uint8_t> read_binary(const std::string& path) {
  std::ifstream stream(path, std::ios::binary | std::ios::ate);
  if (!stream) throw std::runtime_error("cannot open model: " + path);
  const auto end = stream.tellg();
  if (end <= 0 || end > static_cast<std::streamoff>(std::numeric_limits<uint32_t>::max())) {
    throw std::runtime_error("model has an invalid size: " + path);
  }
  std::vector<uint8_t> bytes(static_cast<size_t>(end));
  stream.seekg(0);
  stream.read(reinterpret_cast<char*>(bytes.data()), end);
  if (!stream) throw std::runtime_error("cannot read model: " + path);
  return bytes;
}

uint16_t read_u16(const uint8_t* data) {
  return static_cast<uint16_t>(data[0]) |
         static_cast<uint16_t>(static_cast<uint16_t>(data[1]) << 8);
}

uint32_t read_u32(const uint8_t* data) {
  return static_cast<uint32_t>(data[0]) |
         (static_cast<uint32_t>(data[1]) << 8) |
         (static_cast<uint32_t>(data[2]) << 16) |
         (static_cast<uint32_t>(data[3]) << 24);
}

AudioData decode_wav(const uint8_t* wav, size_t size) {
  if (wav == nullptr || size < 44 || std::memcmp(wav, "RIFF", 4) != 0 ||
      std::memcmp(wav + 8, "WAVE", 4) != 0) {
    throw std::runtime_error("input is not a RIFF/WAVE file");
  }

  const uint8_t* format_data = nullptr;
  size_t format_size = 0;
  const uint8_t* sample_data = nullptr;
  size_t sample_size = 0;
  for (size_t offset = 12; offset + 8 <= size;) {
    const uint32_t chunk_size = read_u32(wav + offset + 4);
    const size_t payload = offset + 8;
    if (payload > size || chunk_size > size - payload) {
      throw std::runtime_error("WAV contains a truncated chunk");
    }
    if (std::memcmp(wav + offset, "fmt ", 4) == 0) {
      format_data = wav + payload;
      format_size = chunk_size;
    } else if (std::memcmp(wav + offset, "data", 4) == 0 && sample_data == nullptr) {
      sample_data = wav + payload;
      sample_size = chunk_size;
    }
    const size_t padded = static_cast<size_t>(chunk_size) + (chunk_size & 1U);
    if (padded > size - payload) break;
    offset = payload + padded;
  }
  if (format_data == nullptr || format_size < 16 || sample_data == nullptr) {
    throw std::runtime_error("WAV is missing fmt or data chunk");
  }

  uint16_t format = read_u16(format_data);
  const uint16_t channels = read_u16(format_data + 2);
  const uint32_t sample_rate = read_u32(format_data + 4);
  const uint16_t block_align = read_u16(format_data + 12);
  const uint16_t bits = read_u16(format_data + 14);
  if (format == 0xfffe && format_size >= 40) {
    format = read_u16(format_data + 24);
  }
  if (channels == 0 || channels > 32 || sample_rate < 4000 || sample_rate > 384000) {
    throw std::runtime_error("WAV channel count or sample rate is unsupported");
  }
  const size_t bytes_per_sample = (bits + 7) / 8;
  if (bytes_per_sample == 0 || block_align < channels * bytes_per_sample ||
      (format != 1 && format != 3) ||
      (format == 3 && bits != 32) ||
      (format == 1 && bits != 8 && bits != 16 && bits != 24 && bits != 32)) {
    throw std::runtime_error("WAV encoding is unsupported; use PCM or Float32");
  }
  const size_t frames = sample_size / block_align;
  if (frames == 0 || frames > 100000000) {
    throw std::runtime_error("WAV has no samples or is too long");
  }

  AudioData audio;
  audio.sample_rate = static_cast<int>(sample_rate);
  audio.samples.resize(frames);
  for (size_t frame = 0; frame < frames; ++frame) {
    const uint8_t* row = sample_data + frame * block_align;
    double sum = 0.0;
    for (uint16_t channel = 0; channel < channels; ++channel) {
      const uint8_t* source = row + channel * bytes_per_sample;
      float value = 0.0f;
      if (format == 3) {
        std::memcpy(&value, source, sizeof(value));
      } else if (bits == 8) {
        value = (static_cast<int>(source[0]) - 128) / 128.0f;
      } else if (bits == 16) {
        const int16_t integer = static_cast<int16_t>(read_u16(source));
        value = integer / 32768.0f;
      } else if (bits == 24) {
        int32_t integer = static_cast<int32_t>(source[0]) |
                          (static_cast<int32_t>(source[1]) << 8) |
                          (static_cast<int32_t>(source[2]) << 16);
        if ((integer & 0x800000) != 0) integer |= ~0xffffff;
        value = integer / 8388608.0f;
      } else {
        const int32_t integer = static_cast<int32_t>(read_u32(source));
        value = static_cast<float>(integer / 2147483648.0);
      }
      if (!std::isfinite(value)) value = 0.0f;
      sum += std::clamp(value, -1.0f, 1.0f);
    }
    audio.samples[frame] = static_cast<float>(sum / channels);
  }
  return audio;
}

std::vector<float> resample_audio(const std::vector<float>& input, int source_rate) {
  if (source_rate == kSampleRate) return input;
  const size_t output_size = static_cast<size_t>(std::llround(
      static_cast<double>(input.size()) * kSampleRate / source_rate));
  if (output_size == 0) throw std::runtime_error("audio is too short after resampling");
  constexpr int kHalfWidth = 32;
  constexpr double kPi = 3.14159265358979323846;
  const double cutoff = std::min(1.0, static_cast<double>(kSampleRate) / source_rate) * 0.95;
  std::vector<float> output(output_size);
  const double ratio = static_cast<double>(source_rate) / kSampleRate;
  for (size_t index = 0; index < output_size; ++index) {
    const double position = index * ratio;
    const int64_t center = static_cast<int64_t>(std::floor(position));
    double weighted = 0.0;
    double weight_sum = 0.0;
    for (int tap = -kHalfWidth + 1; tap <= kHalfWidth; ++tap) {
      const int64_t source = center + tap;
      if (source < 0 || source >= static_cast<int64_t>(input.size())) continue;
      const double distance = position - source;
      const double scaled = cutoff * distance;
      const double sinc = std::abs(scaled) < 1.0e-12
                              ? 1.0
                              : std::sin(kPi * scaled) / (kPi * scaled);
      const double normalized = std::abs(distance) / kHalfWidth;
      if (normalized >= 1.0) continue;
      const double window = 0.42 + 0.5 * std::cos(kPi * normalized) +
                            0.08 * std::cos(2.0 * kPi * normalized);
      const double weight = cutoff * sinc * window;
      weighted += input[static_cast<size_t>(source)] * weight;
      weight_sum += weight;
    }
    output[index] = weight_sum == 0.0 ? 0.0f : static_cast<float>(weighted / weight_sum);
  }
  return output;
}

class RknnModel {
 public:
  RknnModel(const std::string& path, int core_mask, const char* label) : label_(label) {
    auto bytes = read_binary(path);
    int result = rknn_init(&context_, bytes.data(), static_cast<uint32_t>(bytes.size()), 0, nullptr);
    if (result != RKNN_SUCC) {
      throw std::runtime_error(label_ + " rknn_init failed: " + std::to_string(result));
    }
    try {
      result = rknn_set_core_mask(context_, static_cast<rknn_core_mask>(core_mask));
      if (result != RKNN_SUCC) {
        throw std::runtime_error(label_ + " rknn_set_core_mask failed: " +
                                 std::to_string(result));
      }
      rknn_input_output_num count{};
      result = rknn_query(context_, RKNN_QUERY_IN_OUT_NUM, &count, sizeof(count));
      if (result != RKNN_SUCC || count.n_input == 0 || count.n_output == 0) {
        throw std::runtime_error(label_ + " has an invalid input/output signature");
      }
      input_attrs_.resize(count.n_input);
      output_attrs_.resize(count.n_output);
      float_inputs_.resize(count.n_input);
      integer_inputs_.resize(count.n_input);
      for (uint32_t index = 0; index < count.n_input; ++index) {
        input_attrs_[index].index = index;
        result = rknn_query(context_, RKNN_QUERY_INPUT_ATTR, &input_attrs_[index],
                            sizeof(rknn_tensor_attr));
        if (result != RKNN_SUCC) throw std::runtime_error(label_ + " input query failed");
        if (input_attrs_[index].type == RKNN_TENSOR_INT64) {
          integer_inputs_[index].resize(input_attrs_[index].n_elems);
        } else if (input_attrs_[index].type == RKNN_TENSOR_FLOAT16 ||
                   input_attrs_[index].type == RKNN_TENSOR_FLOAT32) {
          float_inputs_[index].resize(input_attrs_[index].n_elems);
        } else {
          throw std::runtime_error(label_ + " has an unsupported input tensor type");
        }
      }
      for (uint32_t index = 0; index < count.n_output; ++index) {
        output_attrs_[index].index = index;
        result = rknn_query(context_, RKNN_QUERY_OUTPUT_ATTR, &output_attrs_[index],
                            sizeof(rknn_tensor_attr));
        if (result != RKNN_SUCC) throw std::runtime_error(label_ + " output query failed");
        if (output_attrs_[index].type != RKNN_TENSOR_INT64 &&
            output_attrs_[index].type != RKNN_TENSOR_FLOAT16 &&
            output_attrs_[index].type != RKNN_TENSOR_FLOAT32) {
          throw std::runtime_error(label_ + " has an unsupported output tensor type");
        }
      }
    } catch (...) {
      rknn_destroy(context_);
      context_ = 0;
      throw;
    }
  }

  ~RknnModel() {
    if (context_ != 0) rknn_destroy(context_);
  }

  RknnModel(const RknnModel&) = delete;
  RknnModel& operator=(const RknnModel&) = delete;

  size_t input_count() const { return input_attrs_.size(); }
  size_t output_count() const { return output_attrs_.size(); }
  const rknn_tensor_attr& input_attr(size_t index) const { return input_attrs_.at(index); }
  rknn_context context() const { return context_; }

  void reset_inputs() {
    for (auto& values : float_inputs_) std::fill(values.begin(), values.end(), 0.0f);
    for (auto& values : integer_inputs_) std::fill(values.begin(), values.end(), 0);
  }

  std::vector<float>& float_input(size_t index) {
    if (index >= float_inputs_.size() || float_inputs_[index].empty()) {
      throw std::runtime_error(label_ + " float input index is invalid");
    }
    return float_inputs_[index];
  }

  std::vector<int64_t>& integer_input(size_t index) {
    if (index >= integer_inputs_.size() || integer_inputs_[index].empty()) {
      throw std::runtime_error(label_ + " integer input index is invalid");
    }
    return integer_inputs_[index];
  }

  ModelOutputs run(uint64_t* inference_us) {
    std::vector<rknn_input> inputs(input_attrs_.size());
    for (size_t index = 0; index < inputs.size(); ++index) {
      inputs[index].index = static_cast<uint32_t>(index);
      inputs[index].fmt = input_attrs_[index].fmt;
      inputs[index].pass_through = 0;
      if (!integer_inputs_[index].empty()) {
        inputs[index].buf = integer_inputs_[index].data();
        inputs[index].size = static_cast<uint32_t>(integer_inputs_[index].size() * sizeof(int64_t));
        inputs[index].type = RKNN_TENSOR_INT64;
      } else {
        inputs[index].buf = float_inputs_[index].data();
        inputs[index].size = static_cast<uint32_t>(float_inputs_[index].size() * sizeof(float));
        inputs[index].type = RKNN_TENSOR_FLOAT32;
      }
    }
    const auto begin = Clock::now();
    int result = rknn_inputs_set(context_, static_cast<uint32_t>(inputs.size()), inputs.data());
    if (result != RKNN_SUCC) {
      throw std::runtime_error(label_ + " rknn_inputs_set failed: " + std::to_string(result));
    }
    result = rknn_run(context_, nullptr);
    if (result != RKNN_SUCC) {
      throw std::runtime_error(label_ + " rknn_run failed: " + std::to_string(result));
    }
    std::vector<rknn_output> outputs(output_attrs_.size());
    for (size_t index = 0; index < outputs.size(); ++index) {
      outputs[index].index = static_cast<uint32_t>(index);
      outputs[index].want_float = output_attrs_[index].type == RKNN_TENSOR_INT64 ? 0 : 1;
      outputs[index].is_prealloc = 0;
    }
    result = rknn_outputs_get(context_, static_cast<uint32_t>(outputs.size()), outputs.data(), nullptr);
    if (result != RKNN_SUCC) {
      throw std::runtime_error(label_ + " rknn_outputs_get failed: " + std::to_string(result));
    }
    *inference_us += elapsed_us(begin, Clock::now());
    ModelOutputs copied;
    copied.floats.resize(outputs.size());
    copied.integers.resize(outputs.size());
    try {
      for (size_t index = 0; index < outputs.size(); ++index) {
        if (output_attrs_[index].type == RKNN_TENSOR_INT64) {
          const auto* values = static_cast<const int64_t*>(outputs[index].buf);
          copied.integers[index].assign(values, values + output_attrs_[index].n_elems);
        } else {
          const auto* values = static_cast<const float*>(outputs[index].buf);
          copied.floats[index].assign(values, values + output_attrs_[index].n_elems);
        }
      }
    } catch (...) {
      rknn_outputs_release(context_, static_cast<uint32_t>(outputs.size()), outputs.data());
      throw;
    }
    rknn_outputs_release(context_, static_cast<uint32_t>(outputs.size()), outputs.data());
    return copied;
  }

 private:
  std::string label_;
  rknn_context context_ = 0;
  std::vector<rknn_tensor_attr> input_attrs_;
  std::vector<rknn_tensor_attr> output_attrs_;
  std::vector<std::vector<float>> float_inputs_;
  std::vector<std::vector<int64_t>> integer_inputs_;
};

std::vector<float> nchw_to_nhwc(const std::vector<float>& source,
                                const rknn_tensor_attr& target) {
  if (target.n_dims != 4) throw std::runtime_error("encoder cache tensor is not rank 4");
  const size_t n = target.dims[0];
  const size_t h = target.dims[1];
  const size_t w = target.dims[2];
  const size_t c = target.dims[3];
  if (n * h * w * c != source.size()) {
    throw std::runtime_error("encoder cache tensor shape is inconsistent");
  }
  std::vector<float> target_data(source.size());
  for (size_t batch = 0; batch < n; ++batch) {
    for (size_t channel = 0; channel < c; ++channel) {
      for (size_t y = 0; y < h; ++y) {
        for (size_t x = 0; x < w; ++x) {
          target_data[((batch * h + y) * w + x) * c + channel] =
              source[((batch * c + channel) * h + y) * w + x];
        }
      }
    }
  }
  return target_data;
}

std::vector<std::string> read_vocabulary(const std::string& path) {
  std::ifstream stream(path);
  if (!stream) throw std::runtime_error("cannot open vocabulary: " + path);
  std::map<size_t, std::string> indexed;
  std::string line;
  while (std::getline(stream, line)) {
    const size_t separator = line.find_last_of(' ');
    if (separator == std::string::npos) continue;
    const size_t id = static_cast<size_t>(std::stoul(line.substr(separator + 1)));
    indexed[id] = line.substr(0, separator);
  }
  if (indexed.empty() || indexed.rbegin()->first < static_cast<size_t>(kJoinerVocabulary - 1)) {
    throw std::runtime_error("vocabulary is incomplete");
  }
  std::vector<std::string> vocabulary(indexed.rbegin()->first + 1);
  for (auto& [id, token] : indexed) vocabulary[id] = std::move(token);
  return vocabulary;
}

std::string replace_sentencepiece_space(std::string token) {
  const std::string marker = "\xE2\x96\x81";
  for (size_t position = 0; (position = token.find(marker, position)) != std::string::npos;) {
    token.replace(position, marker.size(), " ");
    ++position;
  }
  return token;
}

float log_add(float left, float right) {
  if (!std::isfinite(left)) return right;
  if (!std::isfinite(right)) return left;
  const float high = std::max(left, right);
  return high + std::log(std::exp(left - high) + std::exp(right - high));
}

std::vector<float> log_softmax(const std::vector<float>& logits) {
  const float high = *std::max_element(logits.begin(), logits.end());
  double sum = 0.0;
  for (float value : logits) sum += std::exp(static_cast<double>(value - high));
  const float normalizer = high + static_cast<float>(std::log(sum));
  std::vector<float> output(logits.size());
  for (size_t index = 0; index < logits.size(); ++index) {
    output[index] = logits[index] - normalizer;
  }
  return output;
}

struct Hypothesis {
  std::vector<int64_t> tokens;
  std::vector<int> timestamps;
  float score = -std::numeric_limits<float>::infinity();
};

struct Candidate {
  Hypothesis hypothesis;
  float path_score = -std::numeric_limits<float>::infinity();
};

class ZipformerEngine {
 public:
  ZipformerEngine(const std::string& encoder_path, const std::string& decoder_path,
                  const std::string& joiner_path, const std::string& vocab_path,
                  int core_mask)
      : encoder_(std::make_unique<RknnModel>(encoder_path, core_mask, "Zipformer encoder")),
        decoder_(std::make_unique<RknnModel>(decoder_path, core_mask, "Zipformer decoder")),
        joiner_(std::make_unique<RknnModel>(joiner_path, core_mask, "Zipformer joiner")),
        vocabulary_(read_vocabulary(vocab_path)) {
    if (encoder_->input_count() != encoder_->output_count() ||
        encoder_->float_input(0).size() != kSegmentFrames * kMelBins ||
        decoder_->input_count() != 1 || decoder_->output_count() != 1 ||
        decoder_->integer_input(0).size() != kContextSize ||
        joiner_->input_count() != 2 || joiner_->output_count() != 1) {
      throw std::runtime_error("Zipformer RKNN model signatures are incompatible");
    }
    rknn_sdk_version versions{};
    if (rknn_query(encoder_->context(), RKNN_QUERY_SDK_VERSION, &versions, sizeof(versions)) ==
        RKNN_SUCC) {
      runtime_version_ = versions.api_version;
      driver_version_ = versions.drv_version;
    }
  }

  const std::string& runtime_version() const { return runtime_version_; }
  const std::string& driver_version() const { return driver_version_; }

  std::string transcribe(const uint8_t* wav, size_t wav_size, int beam_size,
                         NativeTiming* timing) {
    *timing = {};
    encoder_->reset_inputs();
    const auto preprocess_begin = Clock::now();
    auto decoded = decode_wav(wav, wav_size);
    auto samples = resample_audio(decoded.samples, decoded.sample_rate);
    timing->audio_duration_ms = static_cast<uint64_t>(
        std::llround(samples.size() * 1000.0 / kSampleRate));

    knf::FbankOptions options;
    options.frame_opts.samp_freq = kSampleRate;
    options.mel_opts.num_bins = kMelBins;
    options.mel_opts.high_freq = -400;
    options.frame_opts.dither = 0;
    options.frame_opts.snip_edges = false;
    knf::OnlineFbank fbank(options);
    fbank.AcceptWaveform(kSampleRate, samples.data(), static_cast<int32_t>(samples.size()));
    const int original_frames = fbank.NumFramesReady();
    if (original_frames <= 0) throw std::runtime_error("audio is too short to extract features");
    const int chunks = (original_frames + kSegmentOffset - 1) / kSegmentOffset;
    const int needed_frames = (chunks - 1) * kSegmentOffset + kSegmentFrames;
    if (needed_frames > original_frames) {
      const int padding_samples = (needed_frames - original_frames) * (kSampleRate / 100);
      std::vector<float> padding(padding_samples);
      fbank.AcceptWaveform(kSampleRate, padding.data(), static_cast<int32_t>(padding.size()));
    }
    fbank.InputFinished();
    timing->preprocess_us = elapsed_us(preprocess_begin, Clock::now());

    Hypothesis initial;
    initial.score = 0.0f;
    std::vector<Hypothesis> hypotheses{std::move(initial)};
    std::map<std::pair<int64_t, int64_t>, std::vector<float>> decoder_cache;
    const auto pipeline_begin = Clock::now();
    for (int chunk = 0; chunk < chunks; ++chunk) {
      auto& encoder_input = encoder_->float_input(0);
      const int frame_start = chunk * kSegmentOffset;
      for (int frame = 0; frame < kSegmentFrames; ++frame) {
        const float* feature = fbank.GetFrame(frame_start + frame);
        std::copy(feature, feature + kMelBins,
                  encoder_input.begin() + static_cast<size_t>(frame) * kMelBins);
      }
      const auto encoder_outputs = encoder_->run(&timing->inference_us);
      if (encoder_outputs.floats[0].size() != kEncoderOutputFrames * kDecoderDimension) {
        throw std::runtime_error("Zipformer encoder output has an unexpected shape");
      }
      update_encoder_cache(encoder_outputs);
      for (int frame = 0; frame < kEncoderOutputFrames; ++frame) {
        const float* encoder_frame = encoder_outputs.floats[0].data() +
                                     static_cast<size_t>(frame) * kDecoderDimension;
        hypotheses = advance_beam(hypotheses, encoder_frame, beam_size,
                                  chunk * kEncoderOutputFrames + frame,
                                  &decoder_cache, &timing->inference_us);
      }
    }
    const uint64_t pipeline_us = elapsed_us(pipeline_begin, Clock::now());
    timing->postprocess_us = pipeline_us > timing->inference_us
                                 ? pipeline_us - timing->inference_us
                                 : 0;
    const auto& best = *std::max_element(
        hypotheses.begin(), hypotheses.end(),
        [](const Hypothesis& left, const Hypothesis& right) { return left.score < right.score; });
    timing->token_count = best.tokens.size();
    return decode_tokens(best.tokens);
  }

 private:
  void update_encoder_cache(const ModelOutputs& outputs) {
    for (size_t index = 1; index < encoder_->input_count(); ++index) {
      const auto& attr = encoder_->input_attr(index);
      if (!outputs.integers[index].empty()) {
        auto& target = encoder_->integer_input(index);
        if (target.size() != outputs.integers[index].size()) {
          throw std::runtime_error("Zipformer integer cache size changed");
        }
        target = outputs.integers[index];
      } else {
        auto values = outputs.floats[index];
        if (attr.fmt == RKNN_TENSOR_NHWC) values = nchw_to_nhwc(values, attr);
        auto& target = encoder_->float_input(index);
        if (target.size() != values.size()) {
          throw std::runtime_error("Zipformer float cache size changed");
        }
        target = std::move(values);
      }
    }
  }

  const std::vector<float>& decoder_output(
      const Hypothesis& hypothesis,
      std::map<std::pair<int64_t, int64_t>, std::vector<float>>* cache,
      uint64_t* inference_us) {
    const size_t count = hypothesis.tokens.size();
    const int64_t previous = count >= 2 ? hypothesis.tokens[count - 2] : kBlankId;
    const int64_t latest = count >= 1 ? hypothesis.tokens[count - 1] : kBlankId;
    const std::pair<int64_t, int64_t> key{previous, latest};
    auto found = cache->find(key);
    if (found != cache->end()) return found->second;
    auto& input = decoder_->integer_input(0);
    input[0] = previous;
    input[1] = latest;
    auto outputs = decoder_->run(inference_us);
    if (outputs.floats[0].size() != kDecoderDimension) {
      throw std::runtime_error("Zipformer decoder output has an unexpected shape");
    }
    return cache->emplace(key, std::move(outputs.floats[0])).first->second;
  }

  std::vector<float> joiner_output(const float* encoder_frame,
                                   const std::vector<float>& decoder_output,
                                   uint64_t* inference_us) {
    auto& encoder_input = joiner_->float_input(0);
    auto& decoder_input = joiner_->float_input(1);
    if (encoder_input.size() != kDecoderDimension ||
        decoder_input.size() != kDecoderDimension) {
      throw std::runtime_error("Zipformer joiner input has an unexpected shape");
    }
    std::copy(encoder_frame, encoder_frame + kDecoderDimension, encoder_input.begin());
    decoder_input = decoder_output;
    auto outputs = joiner_->run(inference_us);
    if (outputs.floats[0].size() != kJoinerVocabulary) {
      throw std::runtime_error("Zipformer joiner output has an unexpected shape");
    }
    return outputs.floats[0];
  }

  std::vector<Hypothesis> advance_beam(
      const std::vector<Hypothesis>& current, const float* encoder_frame,
      int beam_size, int timestamp,
      std::map<std::pair<int64_t, int64_t>, std::vector<float>>* decoder_cache,
      uint64_t* inference_us) {
    std::map<std::vector<int64_t>, Candidate> merged;
    for (const auto& hypothesis : current) {
      const auto& decoder = decoder_output(hypothesis, decoder_cache, inference_us);
      const auto probabilities = log_softmax(
          joiner_output(encoder_frame, decoder, inference_us));

      Candidate blank;
      blank.hypothesis = hypothesis;
      blank.path_score = hypothesis.score + probabilities[kBlankId];
      blank.hypothesis.score = blank.path_score;
      merge_candidate(&merged, std::move(blank));

      std::vector<int64_t> ids(kJoinerVocabulary - 2);
      size_t cursor = 0;
      for (int64_t id = 1; id < kJoinerVocabulary; ++id) {
        if (id != kUnknownId) ids[cursor++] = id;
      }
      ids.resize(cursor);
      const size_t keep = std::min(ids.size(), static_cast<size_t>(beam_size));
      std::partial_sort(ids.begin(), ids.begin() + keep, ids.end(),
                        [&probabilities](int64_t left, int64_t right) {
                          return probabilities[left] > probabilities[right];
                        });
      for (size_t index = 0; index < keep; ++index) {
        const int64_t id = ids[index];
        Candidate emitted;
        emitted.hypothesis = hypothesis;
        emitted.hypothesis.tokens.push_back(id);
        emitted.hypothesis.timestamps.push_back(timestamp);
        emitted.path_score = hypothesis.score + probabilities[id];
        emitted.hypothesis.score = emitted.path_score;
        merge_candidate(&merged, std::move(emitted));
      }
    }

    std::vector<Hypothesis> next;
    next.reserve(merged.size());
    for (auto& [_, candidate] : merged) next.push_back(std::move(candidate.hypothesis));
    const size_t keep = std::min(next.size(), static_cast<size_t>(beam_size));
    std::partial_sort(next.begin(), next.begin() + keep, next.end(),
                      [](const Hypothesis& left, const Hypothesis& right) {
                        return left.score > right.score;
                      });
    next.resize(keep);
    return next;
  }

  static void merge_candidate(
      std::map<std::vector<int64_t>, Candidate>* merged, Candidate candidate) {
    auto found = merged->find(candidate.hypothesis.tokens);
    if (found == merged->end()) {
      merged->emplace(candidate.hypothesis.tokens, std::move(candidate));
      return;
    }
    const float combined = log_add(found->second.hypothesis.score,
                                   candidate.hypothesis.score);
    if (candidate.path_score > found->second.path_score) {
      candidate.hypothesis.score = combined;
      found->second = std::move(candidate);
    } else {
      found->second.hypothesis.score = combined;
    }
  }

  std::string decode_tokens(const std::vector<int64_t>& tokens) const {
    std::string text;
    for (int64_t id : tokens) {
      if (id <= kUnknownId || id >= static_cast<int64_t>(vocabulary_.size())) continue;
      text += replace_sentencepiece_space(vocabulary_[id]);
    }
    const size_t first = text.find_first_not_of(" \t\r\n");
    if (first == std::string::npos) return {};
    const size_t last = text.find_last_not_of(" \t\r\n");
    return text.substr(first, last - first + 1);
  }

  std::unique_ptr<RknnModel> encoder_;
  std::unique_ptr<RknnModel> decoder_;
  std::unique_ptr<RknnModel> joiner_;
  std::vector<std::string> vocabulary_;
  std::string runtime_version_ = "unknown";
  std::string driver_version_ = "unknown";
};

}  // namespace

extern "C" ZipformerEngine* rkserve_zipformer_asr_create(
    const char* encoder_path, const char* decoder_path, const char* joiner_path,
    const char* vocab_path, int core_mask, char* error, size_t error_capacity) {
  try {
    if (encoder_path == nullptr || decoder_path == nullptr || joiner_path == nullptr ||
        vocab_path == nullptr || core_mask <= 0) {
      throw std::runtime_error("Zipformer model paths and NPU core mask are required");
    }
    return new ZipformerEngine(encoder_path, decoder_path, joiner_path, vocab_path, core_mask);
  } catch (const std::exception& exception) {
    write_error(error, error_capacity, exception.what());
    return nullptr;
  }
}

extern "C" void rkserve_zipformer_asr_destroy(ZipformerEngine* engine) {
  delete engine;
}

extern "C" int rkserve_zipformer_asr_transcribe(
    ZipformerEngine* engine, const uint8_t* wav, size_t wav_size, int beam_size,
    char** text, size_t* text_size, NativeTiming* timing,
    char* error, size_t error_capacity) {
  try {
    if (engine == nullptr || wav == nullptr || wav_size == 0 || beam_size < 1 ||
        beam_size > 8 || text == nullptr || text_size == nullptr || timing == nullptr) {
      throw std::runtime_error("invalid Zipformer transcription request");
    }
    *text = nullptr;
    *text_size = 0;
    const std::string result = engine->transcribe(wav, wav_size, beam_size, timing);
    auto* allocated = static_cast<char*>(std::malloc(result.size() + 1));
    if (allocated == nullptr) throw std::bad_alloc();
    std::memcpy(allocated, result.data(), result.size());
    allocated[result.size()] = '\0';
    *text = allocated;
    *text_size = result.size();
    return 0;
  } catch (const std::exception& exception) {
    write_error(error, error_capacity, exception.what());
    return -1;
  }
}

extern "C" void rkserve_zipformer_asr_free(void* pointer) {
  std::free(pointer);
}

extern "C" int rkserve_zipformer_asr_versions(
    ZipformerEngine* engine, char* runtime, size_t runtime_capacity,
    char* driver, size_t driver_capacity) {
  if (engine == nullptr) return -1;
  write_error(runtime, runtime_capacity, engine->runtime_version());
  write_error(driver, driver_capacity, engine->driver_version());
  return 0;
}
