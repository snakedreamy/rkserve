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
#include <set>
#include <sstream>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

#include "kaldi-native-fbank/csrc/online-feature.h"
#include "rknn_api.h"

namespace {

constexpr int kSampleRate = 16000;
constexpr int kMelBins = 80;
constexpr int kLfrWindow = 7;
constexpr int kLfrShift = 6;
constexpr int kLfrDimension = 560;
constexpr int kFixedFrames = 344;
constexpr int kPromptFrames = 4;
constexpr int kVocabularySize = 25055;
constexpr int kBlankId = 0;
// The fixed encoder can represent roughly 20 seconds, but RK3576 FP16 becomes
// numerically unstable for some longer Mandarin inputs. Keep each inference at
// or below 10 seconds and search a wide interval for a low-energy boundary.
constexpr int kTargetSegmentSamples = 8 * kSampleRate;
constexpr int kMaximumSegmentSamples = 10 * kSampleRate;

using Clock = std::chrono::steady_clock;

struct NativeTiming {
  uint64_t preprocess_us;
  uint64_t inference_us;
  uint64_t postprocess_us;
  uint64_t audio_duration_ms;
  uint64_t segment_count;
};

struct AudioData {
  std::vector<float> samples;
  int sample_rate = 0;
};

struct SegmentRange {
  size_t start = 0;
  size_t end = 0;
};

struct DecodedSegment {
  std::string text;
  std::string language;
  std::string emotion;
  std::vector<std::string> events;
  uint64_t start_ms = 0;
  uint64_t end_ms = 0;
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
  if (!stream) throw std::runtime_error("cannot open asset: " + path);
  const auto end = stream.tellg();
  if (end <= 0 || end > static_cast<std::streamoff>(std::numeric_limits<uint32_t>::max())) {
    throw std::runtime_error("asset has an invalid size: " + path);
  }
  std::vector<uint8_t> bytes(static_cast<size_t>(end));
  stream.seekg(0);
  stream.read(reinterpret_cast<char*>(bytes.data()), end);
  if (!stream) throw std::runtime_error("cannot read asset: " + path);
  return bytes;
}

std::string read_text(const std::string& path) {
  std::ifstream stream(path);
  if (!stream) throw std::runtime_error("cannot open text asset: " + path);
  return std::string(std::istreambuf_iterator<char>(stream),
                     std::istreambuf_iterator<char>());
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
  if (format == 0xfffe && format_size >= 40) format = read_u16(format_data + 24);
  if (channels == 0 || channels > 32 || sample_rate < 4000 || sample_rate > 384000) {
    throw std::runtime_error("WAV channel count or sample rate is unsupported");
  }
  const size_t bytes_per_sample = (bits + 7) / 8;
  if (bytes_per_sample == 0 || block_align < channels * bytes_per_sample ||
      (format != 1 && format != 3) || (format == 3 && bits != 32) ||
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
        value = static_cast<int16_t>(read_u16(source)) / 32768.0f;
      } else if (bits == 24) {
        int32_t integer = static_cast<int32_t>(source[0]) |
                          (static_cast<int32_t>(source[1]) << 8) |
                          (static_cast<int32_t>(source[2]) << 16);
        if ((integer & 0x800000) != 0) integer |= ~0xffffff;
        value = integer / 8388608.0f;
      } else {
        value = static_cast<float>(static_cast<int32_t>(read_u32(source)) /
                                   2147483648.0);
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
  const double cutoff =
      std::min(1.0, static_cast<double>(kSampleRate) / source_rate) * 0.95;
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
    output[index] =
        weight_sum == 0.0 ? 0.0f : static_cast<float>(weighted / weight_sum);
  }
  return output;
}

std::vector<SegmentRange> split_audio(const std::vector<float>& samples) {
  std::vector<SegmentRange> ranges;
  size_t start = 0;
  constexpr size_t kEnergyWindow = kSampleRate / 5;
  constexpr size_t kEnergyHop = kSampleRate / 50;
  constexpr size_t kSearchRadius = 3 * kSampleRate;
  while (samples.size() - start > static_cast<size_t>(kMaximumSegmentSamples)) {
    const size_t target = start + kTargetSegmentSamples;
    const size_t search_begin = target > kSearchRadius ? target - kSearchRadius : start;
    const size_t search_end = std::min(
        {target + kSearchRadius, start + static_cast<size_t>(kMaximumSegmentSamples),
         samples.size()});
    size_t best = target;
    double best_energy = std::numeric_limits<double>::infinity();
    for (size_t candidate = search_begin;
         candidate + kEnergyWindow <= search_end; candidate += kEnergyHop) {
      double energy = 0.0;
      for (size_t index = candidate; index < candidate + kEnergyWindow; ++index) {
        energy += static_cast<double>(samples[index]) * samples[index];
      }
      if (energy < best_energy) {
        best_energy = energy;
        best = candidate + kEnergyWindow / 2;
      }
    }
    if (best <= start || best - start > static_cast<size_t>(kMaximumSegmentSamples)) {
      best = start + kTargetSegmentSamples;
    }
    ranges.push_back({start, best});
    start = best;
  }
  ranges.push_back({start, samples.size()});
  return ranges;
}

std::pair<std::vector<float>, std::vector<float>> read_cmvn(const std::string& path) {
  const std::string content = read_text(path);
  std::vector<std::vector<float>> groups;
  for (size_t position = 0;;) {
    const size_t begin = content.find('[', position);
    if (begin == std::string::npos) break;
    const size_t end = content.find(']', begin + 1);
    if (end == std::string::npos) break;
    std::istringstream values(content.substr(begin + 1, end - begin - 1));
    std::vector<float> group;
    float value = 0.0f;
    while (values >> value) group.push_back(value);
    if (group.size() == kLfrDimension) groups.push_back(std::move(group));
    position = end + 1;
  }
  if (groups.size() < 2) throw std::runtime_error("CMVN asset is incomplete");
  return {std::move(groups[0]), std::move(groups[1])};
}

std::vector<float> read_prompt_embeddings(const std::string& path) {
  const auto bytes = read_binary(path);
  if (bytes.size() < 16 || std::memcmp(bytes.data(), "\x93NUMPY", 6) != 0) {
    throw std::runtime_error("prompt embedding is not a NumPy array");
  }
  const uint8_t major = bytes[6];
  size_t header_length = 0;
  size_t header_offset = 0;
  if (major == 1) {
    header_length = read_u16(bytes.data() + 8);
    header_offset = 10;
  } else if (major == 2 || major == 3) {
    header_length = read_u32(bytes.data() + 8);
    header_offset = 12;
  } else {
    throw std::runtime_error("prompt embedding uses an unsupported NumPy version");
  }
  if (header_offset + header_length > bytes.size()) {
    throw std::runtime_error("prompt embedding has a truncated header");
  }
  const std::string header(reinterpret_cast<const char*>(bytes.data() + header_offset),
                           header_length);
  if (header.find("(16, 560)") == std::string::npos ||
      (header.find("'<f4'") == std::string::npos &&
       header.find("'|f4'") == std::string::npos)) {
    throw std::runtime_error("prompt embedding shape or type is incompatible");
  }
  const size_t data_offset = header_offset + header_length;
  constexpr size_t kValues = 16 * kLfrDimension;
  if (bytes.size() - data_offset != kValues * sizeof(float)) {
    throw std::runtime_error("prompt embedding data size is incompatible");
  }
  std::vector<float> output(kValues);
  std::memcpy(output.data(), bytes.data() + data_offset, output.size() * sizeof(float));
  return output;
}

std::vector<std::string> read_tokens(const std::string& path) {
  std::ifstream stream(path);
  if (!stream) throw std::runtime_error("cannot open token table: " + path);
  std::map<size_t, std::string> indexed;
  std::string line;
  while (std::getline(stream, line)) {
    const size_t separator = line.find_last_of(' ');
    if (separator == std::string::npos) continue;
    const size_t id = static_cast<size_t>(std::stoul(line.substr(separator + 1)));
    indexed[id] = line.substr(0, separator);
  }
  if (indexed.size() != kVocabularySize ||
      indexed.rbegin()->first != static_cast<size_t>(kVocabularySize - 1)) {
    throw std::runtime_error("token table is incomplete");
  }
  std::vector<std::string> tokens(kVocabularySize);
  for (auto& [id, token] : indexed) tokens[id] = std::move(token);
  return tokens;
}

std::string replace_sentencepiece_space(std::string token) {
  const std::string marker = "\xE2\x96\x81";
  for (size_t position = 0;
       (position = token.find(marker, position)) != std::string::npos;) {
    token.replace(position, marker.size(), " ");
    ++position;
  }
  return token;
}

std::string trim(std::string value) {
  const size_t first = value.find_first_not_of(" \t\r\n");
  if (first == std::string::npos) return {};
  const size_t last = value.find_last_not_of(" \t\r\n");
  return value.substr(first, last - first + 1);
}

std::string json_escape(const std::string& value) {
  std::string output;
  output.reserve(value.size() + 8);
  constexpr char kHex[] = "0123456789abcdef";
  for (unsigned char byte : value) {
    switch (byte) {
      case '"': output += "\\\""; break;
      case '\\': output += "\\\\"; break;
      case '\b': output += "\\b"; break;
      case '\f': output += "\\f"; break;
      case '\n': output += "\\n"; break;
      case '\r': output += "\\r"; break;
      case '\t': output += "\\t"; break;
      default:
        if (byte < 0x20) {
          output += "\\u00";
          output += kHex[byte >> 4];
          output += kHex[byte & 0x0f];
        } else {
          output.push_back(static_cast<char>(byte));
        }
    }
  }
  return output;
}

bool is_ascii_word_edge(char value) {
  const unsigned char byte = static_cast<unsigned char>(value);
  return (byte >= 'A' && byte <= 'Z') || (byte >= 'a' && byte <= 'z') ||
         (byte >= '0' && byte <= '9');
}

void append_segment_text(std::string* text, const std::string& segment) {
  if (segment.empty()) return;
  if (!text->empty() && is_ascii_word_edge(text->back()) &&
      is_ascii_word_edge(segment.front())) {
    text->push_back(' ');
  }
  *text += segment;
}

class SenseVoiceModel {
 public:
  SenseVoiceModel(const std::string& path, int core_mask) {
    auto bytes = read_binary(path);
    int result = rknn_init(&context_, bytes.data(), static_cast<uint32_t>(bytes.size()),
                           0, nullptr);
    if (result != RKNN_SUCC) {
      throw std::runtime_error("SenseVoice rknn_init failed: " +
                               std::to_string(result));
    }
    try {
      result = rknn_set_core_mask(context_, static_cast<rknn_core_mask>(core_mask));
      if (result != RKNN_SUCC) {
        throw std::runtime_error("SenseVoice rknn_set_core_mask failed: " +
                                 std::to_string(result));
      }
      rknn_input_output_num count{};
      result = rknn_query(context_, RKNN_QUERY_IN_OUT_NUM, &count, sizeof(count));
      if (result != RKNN_SUCC || count.n_input != 1 || count.n_output != 1) {
        throw std::runtime_error("SenseVoice model signature is incompatible");
      }
      input_attr_.index = 0;
      output_attr_.index = 0;
      if (rknn_query(context_, RKNN_QUERY_INPUT_ATTR, &input_attr_,
                     sizeof(input_attr_)) != RKNN_SUCC ||
          rknn_query(context_, RKNN_QUERY_OUTPUT_ATTR, &output_attr_,
                     sizeof(output_attr_)) != RKNN_SUCC) {
        throw std::runtime_error("SenseVoice tensor query failed");
      }
      if (input_attr_.n_elems != kFixedFrames * kLfrDimension ||
          output_attr_.n_elems != kFixedFrames * kVocabularySize) {
        throw std::runtime_error("SenseVoice tensor dimensions are incompatible");
      }
      input_.resize(input_attr_.n_elems);
      rknn_sdk_version versions{};
      if (rknn_query(context_, RKNN_QUERY_SDK_VERSION, &versions,
                     sizeof(versions)) == RKNN_SUCC) {
        runtime_version_ = versions.api_version;
        driver_version_ = versions.drv_version;
      }
    } catch (...) {
      rknn_destroy(context_);
      context_ = 0;
      throw;
    }
  }

  ~SenseVoiceModel() {
    if (context_ != 0) rknn_destroy(context_);
  }

  SenseVoiceModel(const SenseVoiceModel&) = delete;
  SenseVoiceModel& operator=(const SenseVoiceModel&) = delete;

  std::vector<float>& input() { return input_; }
  const std::string& runtime_version() const { return runtime_version_; }
  const std::string& driver_version() const { return driver_version_; }

  std::vector<float> run(uint64_t* inference_us) {
    rknn_input input{};
    input.index = 0;
    input.buf = input_.data();
    input.size = static_cast<uint32_t>(input_.size() * sizeof(float));
    input.type = RKNN_TENSOR_FLOAT32;
    input.fmt = input_attr_.fmt;
    input.pass_through = 0;
    const auto begin = Clock::now();
    int result = rknn_inputs_set(context_, 1, &input);
    if (result == RKNN_SUCC) result = rknn_run(context_, nullptr);
    if (result != RKNN_SUCC) {
      throw std::runtime_error("SenseVoice NPU inference failed: " +
                               std::to_string(result));
    }
    rknn_output output{};
    output.index = 0;
    output.want_float = 1;
    output.is_prealloc = 0;
    result = rknn_outputs_get(context_, 1, &output, nullptr);
    if (result != RKNN_SUCC) {
      throw std::runtime_error("SenseVoice output retrieval failed: " +
                               std::to_string(result));
    }
    const size_t expected_bytes =
        static_cast<size_t>(output_attr_.n_elems) * sizeof(float);
    if (output.buf == nullptr || output.size < expected_bytes) {
      rknn_outputs_release(context_, 1, &output);
      throw std::runtime_error("SenseVoice output buffer is incomplete");
    }
    *inference_us += elapsed_us(begin, Clock::now());
    try {
      std::vector<float> copied(output_attr_.n_elems);
      std::memcpy(copied.data(), output.buf, copied.size() * sizeof(float));
      rknn_outputs_release(context_, 1, &output);
      return copied;
    } catch (...) {
      rknn_outputs_release(context_, 1, &output);
      throw;
    }
  }

 private:
  rknn_context context_ = 0;
  rknn_tensor_attr input_attr_{};
  rknn_tensor_attr output_attr_{};
  std::vector<float> input_;
  std::string runtime_version_ = "unknown";
  std::string driver_version_ = "unknown";
};

class SenseVoiceEngine {
 public:
  SenseVoiceEngine(const std::string& model_path, const std::string& cmvn_path,
                   const std::string& embedding_path,
                   const std::string& tokens_path, int core_mask)
      : model_(std::make_unique<SenseVoiceModel>(model_path, core_mask)),
        embeddings_(read_prompt_embeddings(embedding_path)),
        tokens_(read_tokens(tokens_path)) {
    auto cmvn = read_cmvn(cmvn_path);
    cmvn_add_ = std::move(cmvn.first);
    cmvn_scale_ = std::move(cmvn.second);
  }

  const std::string& runtime_version() const { return model_->runtime_version(); }
  const std::string& driver_version() const { return model_->driver_version(); }

  std::string transcribe(const uint8_t* wav, size_t wav_size,
                         const std::string& language, bool with_itn,
                         NativeTiming* timing) {
    *timing = {};
    const auto initial_preprocess = Clock::now();
    auto decoded = decode_wav(wav, wav_size);
    auto samples = resample_audio(decoded.samples, decoded.sample_rate);
    timing->audio_duration_ms = static_cast<uint64_t>(
        std::llround(samples.size() * 1000.0 / kSampleRate));
    const auto ranges = split_audio(samples);
    timing->segment_count = ranges.size();
    timing->preprocess_us += elapsed_us(initial_preprocess, Clock::now());

    std::vector<DecodedSegment> segments;
    segments.reserve(ranges.size());
    for (const auto& range : ranges) {
      const auto preprocess_begin = Clock::now();
      const int valid = build_input(samples.data() + range.start,
                                    range.end - range.start, language, with_itn);
      timing->preprocess_us += elapsed_us(preprocess_begin, Clock::now());
      auto logits = model_->run(&timing->inference_us);
      const auto postprocess_begin = Clock::now();
      auto segment = decode_logits(logits, valid, language);
      segment.start_ms = range.start * 1000 / kSampleRate;
      segment.end_ms = range.end * 1000 / kSampleRate;
      segments.push_back(std::move(segment));
      timing->postprocess_us += elapsed_us(postprocess_begin, Clock::now());
    }

    const auto json_begin = Clock::now();
    std::string full_text;
    std::string detected_language;
    std::string emotion;
    std::set<std::string> events;
    for (const auto& segment : segments) {
      append_segment_text(&full_text, segment.text);
      if (detected_language.empty() && !segment.language.empty()) {
        detected_language = segment.language;
      }
      if (emotion.empty() && !segment.emotion.empty()) emotion = segment.emotion;
      events.insert(segment.events.begin(), segment.events.end());
    }
    std::ostringstream json;
    json << "{\"text\":\"" << json_escape(full_text) << "\",\"language\":";
    if (detected_language.empty()) json << "null";
    else json << "\"" << json_escape(detected_language) << "\"";
    json << ",\"emotion\":";
    if (emotion.empty()) json << "null";
    else json << "\"" << json_escape(emotion) << "\"";
    json << ",\"events\":[";
    size_t event_index = 0;
    for (const auto& event : events) {
      if (event_index++ != 0) json << ',';
      json << "\"" << json_escape(event) << "\"";
    }
    json << "],\"audio_duration_ms\":" << timing->audio_duration_ms
         << ",\"segments\":[";
    for (size_t index = 0; index < segments.size(); ++index) {
      if (index != 0) json << ',';
      const auto& segment = segments[index];
      json << "{\"index\":" << index << ",\"start_ms\":" << segment.start_ms
           << ",\"end_ms\":" << segment.end_ms << ",\"text\":\""
           << json_escape(segment.text) << "\",\"language\":";
      if (segment.language.empty()) json << "null";
      else json << "\"" << json_escape(segment.language) << "\"";
      json << ",\"emotion\":";
      if (segment.emotion.empty()) json << "null";
      else json << "\"" << json_escape(segment.emotion) << "\"";
      json << ",\"events\":[";
      for (size_t item = 0; item < segment.events.size(); ++item) {
        if (item != 0) json << ',';
        json << "\"" << json_escape(segment.events[item]) << "\"";
      }
      json << "]}";
    }
    json << "]}";
    timing->postprocess_us += elapsed_us(json_begin, Clock::now());
    return json.str();
  }

 private:
  static int language_embedding(const std::string& language) {
    if (language == "zh") return 3;
    if (language == "en") return 4;
    if (language == "yue") return 7;
    if (language == "ja") return 11;
    if (language == "ko") return 12;
    return 0;
  }

  int build_input(const float* audio, size_t sample_count,
                  const std::string& language, bool with_itn) {
    knf::FbankOptions options;
    options.frame_opts.samp_freq = kSampleRate;
    options.frame_opts.dither = 0.0f;
    options.frame_opts.window_type = "hamming";
    options.frame_opts.snip_edges = true;
    options.mel_opts.num_bins = kMelBins;
    knf::OnlineFbank fbank(options);
    if (sample_count > static_cast<size_t>(kMaximumSegmentSamples)) {
      throw std::runtime_error("internal SenseVoice segment exceeds 10 seconds");
    }
    // Real-device FP16 output is sensitive to the feature length even though
    // the RKNN tensor itself is fixed. Pad every segment with waveform silence
    // to the same 10-second front-end length so short and boundary segments use
    // an identical numerical path.
    std::vector<float> scaled(kMaximumSegmentSamples, 0.0f);
    for (size_t index = 0; index < sample_count; ++index) {
      scaled[index] = audio[index] * 32768.0f;
    }
    fbank.AcceptWaveform(kSampleRate, scaled.data(), static_cast<int32_t>(scaled.size()));
    fbank.InputFinished();
    const int feature_frames = fbank.NumFramesReady();
    if (feature_frames <= 0) throw std::runtime_error("audio is too short to extract features");
    const int lfr_frames = (feature_frames + kLfrShift - 1) / kLfrShift;
    const int valid = std::min(kFixedFrames, kPromptFrames + lfr_frames);
    auto& input = model_->input();
    std::fill(input.begin(), input.end(), 0.0f);
    const int prompt_ids[4] = {language_embedding(language), 1, 2,
                               with_itn ? 14 : 15};
    for (int row = 0; row < kPromptFrames; ++row) {
      const float* source = embeddings_.data() + prompt_ids[row] * kLfrDimension;
      std::copy(source, source + kLfrDimension,
                input.begin() + row * kLfrDimension);
    }
    const int left_pad = (kLfrWindow - 1) / 2;
    for (int row = 0; row < valid - kPromptFrames; ++row) {
      float* target = input.data() + (row + kPromptFrames) * kLfrDimension;
      for (int window = 0; window < kLfrWindow; ++window) {
        int frame = row * kLfrShift + window - left_pad;
        frame = std::clamp(frame, 0, feature_frames - 1);
        const float* source = fbank.GetFrame(frame);
        for (int mel = 0; mel < kMelBins; ++mel) {
          const int dimension = window * kMelBins + mel;
          target[dimension] =
              (source[mel] + cmvn_add_[dimension]) * cmvn_scale_[dimension];
        }
      }
    }
    return valid;
  }

  static std::string special_value(const std::string& token) {
    if (token.size() >= 4 && token.rfind("<|", 0) == 0 &&
        token.substr(token.size() - 2) == "|>") {
      return token.substr(2, token.size() - 4);
    }
    return {};
  }

  DecodedSegment decode_logits(const std::vector<float>& logits, int valid,
                               const std::string& requested_language) const {
    DecodedSegment result;
    int previous = -1;
    std::set<std::string> events;
    for (int frame = 0; frame < valid; ++frame) {
      const float* row = logits.data() + static_cast<size_t>(frame) * kVocabularySize;
      const int id = static_cast<int>(
          std::max_element(row, row + kVocabularySize) - row);
      if (id == previous) continue;
      previous = id;
      if (id == kBlankId) continue;
      const std::string& token = tokens_[id];
      const std::string special = special_value(token);
      if (special.empty()) {
        result.text += replace_sentencepiece_space(token);
        continue;
      }
      if (id >= 24884 && id <= 24988 && result.language.empty()) {
        result.language = special;
      } else if (id >= 25001 && id <= 25009 && result.emotion.empty()) {
        result.emotion = special;
      } else if ((id >= 24992 && id <= 25000) || (id >= 25010 && id <= 25015)) {
        if (!special.empty() && special.front() != '/' && special != "nospeech") {
          events.insert(special);
        }
      }
    }
    result.text = trim(std::move(result.text));
    if (result.language.empty() && requested_language != "auto") {
      result.language = requested_language;
    }
    result.events.assign(events.begin(), events.end());
    if (result.text.empty()) result.language.clear();
    if (result.text.empty() && result.events.empty()) result.emotion.clear();
    return result;
  }

  std::unique_ptr<SenseVoiceModel> model_;
  std::vector<float> cmvn_add_;
  std::vector<float> cmvn_scale_;
  std::vector<float> embeddings_;
  std::vector<std::string> tokens_;
};

}  // namespace

extern "C" SenseVoiceEngine* rkserve_sensevoice_asr_create(
    const char* model_path, const char* cmvn_path, const char* embedding_path,
    const char* tokens_path, int core_mask, char* error, size_t error_capacity) {
  try {
    if (model_path == nullptr || cmvn_path == nullptr || embedding_path == nullptr ||
        tokens_path == nullptr || core_mask <= 0) {
      throw std::runtime_error("SenseVoice asset paths and NPU core mask are required");
    }
    return new SenseVoiceEngine(model_path, cmvn_path, embedding_path, tokens_path,
                                core_mask);
  } catch (const std::exception& exception) {
    write_error(error, error_capacity, exception.what());
    return nullptr;
  }
}

extern "C" void rkserve_sensevoice_asr_destroy(SenseVoiceEngine* engine) {
  delete engine;
}

extern "C" int rkserve_sensevoice_asr_transcribe(
    SenseVoiceEngine* engine, const uint8_t* wav, size_t wav_size,
    const char* language, int with_itn, char** json, size_t* json_size,
    NativeTiming* timing, char* error, size_t error_capacity) {
  try {
    if (engine == nullptr || wav == nullptr || wav_size == 0 || language == nullptr ||
        json == nullptr || json_size == nullptr || timing == nullptr) {
      throw std::runtime_error("invalid SenseVoice transcription request");
    }
    *json = nullptr;
    *json_size = 0;
    const std::string result =
        engine->transcribe(wav, wav_size, language, with_itn != 0, timing);
    auto* allocated = static_cast<char*>(std::malloc(result.size() + 1));
    if (allocated == nullptr) throw std::bad_alloc();
    std::memcpy(allocated, result.data(), result.size());
    allocated[result.size()] = '\0';
    *json = allocated;
    *json_size = result.size();
    return 0;
  } catch (const std::exception& exception) {
    write_error(error, error_capacity, exception.what());
    return -1;
  }
}

extern "C" void rkserve_sensevoice_asr_free(void* pointer) {
  std::free(pointer);
}

extern "C" int rkserve_sensevoice_asr_versions(
    SenseVoiceEngine* engine, char* runtime, size_t runtime_capacity,
    char* driver, size_t driver_capacity) {
  if (engine == nullptr) return -1;
  write_error(runtime, runtime_capacity, engine->runtime_version());
  write_error(driver, driver_capacity, engine->driver_version());
  return 0;
}
