// SPDX-License-Identifier: GPL-3.0-only

#include <algorithm>
#include <array>
#include <chrono>
#include <cmath>
#include <complex>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <memory>
#include <numeric>
#include <sstream>
#include <stdexcept>
#include <string>
#include <unordered_map>
#include <utility>
#include <vector>

#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>
#include <dlfcn.h>

#include "onnxruntime_c_api.h"
#include "rknn_api.h"

namespace {

using Clock = std::chrono::steady_clock;

constexpr int kSampleRate = 16000;
constexpr int kSeqLength = 80;
constexpr int kMaxTokens = 64;
constexpr int kMelChannels = 80;
constexpr int kMatchaFrames = 599;
constexpr int kVocosFrames = 600;
constexpr int kFftSize = 1024;
constexpr int kFftBins = 513;
constexpr int kHopLength = 256;

static void write_error(char* buffer, size_t capacity, const std::string& message) {
    if (buffer != nullptr && capacity > 0) std::snprintf(buffer, capacity, "%s", message.c_str());
}

static uint64_t elapsed_us(Clock::time_point begin) {
    return static_cast<uint64_t>(
        std::chrono::duration_cast<std::chrono::microseconds>(Clock::now() - begin).count());
}

static std::vector<std::string> split_ws(const std::string& line) {
    std::istringstream stream(line);
    std::vector<std::string> fields;
    for (std::string field; stream >> field;) fields.push_back(std::move(field));
    return fields;
}

static std::vector<std::string> utf8_chars(const std::string& text) {
    std::vector<std::string> result;
    for (size_t offset = 0; offset < text.size();) {
        const unsigned char lead = static_cast<unsigned char>(text[offset]);
        size_t length = 1;
        if ((lead & 0xe0) == 0xc0) length = 2;
        else if ((lead & 0xf0) == 0xe0) length = 3;
        else if ((lead & 0xf8) == 0xf0) length = 4;
        if (offset + length > text.size()) throw std::runtime_error("invalid UTF-8 input");
        for (size_t index = 1; index < length; ++index) {
            if ((static_cast<unsigned char>(text[offset + index]) & 0xc0) != 0x80) {
                throw std::runtime_error("invalid UTF-8 input");
            }
        }
        result.emplace_back(text.substr(offset, length));
        offset += length;
    }
    return result;
}

static bool ascii_letter(const std::string& value) {
    return value.size() == 1 && ((value[0] >= 'a' && value[0] <= 'z') ||
                                 (value[0] >= 'A' && value[0] <= 'Z'));
}

static bool ascii_digit(const std::string& value) {
    return value.size() == 1 && value[0] >= '0' && value[0] <= '9';
}

static bool ascii_word_part(const std::string& value) {
    return ascii_letter(value) || ascii_digit(value) || value == "'" || value == "-";
}

static bool whitespace(const std::string& value) {
    return value == " " || value == "\t" || value == "\r" || value == "\n";
}

static std::string lower_ascii(std::string value) {
    for (char& character : value) {
        if (character >= 'A' && character <= 'Z') character += 'a' - 'A';
    }
    return value;
}

static std::string normalize_punctuation(const std::string& value) {
    static const std::unordered_map<std::string, std::string> replacements = {
        {"，", ","}, {"、", ","}, {"；", ";"}, {"：", ","}, {":", ","},
        {"。", "."}, {"？", "?"}, {"！", "!"}, {"“", "\""}, {"”", "\""},
        {"‘", "'"}, {"’", "'"}, {"（", "("}, {"）", ")"}, {"—", "—"},
    };
    const auto found = replacements.find(value);
    return found == replacements.end() ? value : found->second;
}

static bool sentence_boundary(const std::string& value) {
    return value == "." || value == "!" || value == "?" || value == ";";
}

static std::string join_chars(
    const std::vector<std::string>& chars, size_t offset, size_t length) {
    std::string result;
    for (size_t i = 0; i < length; ++i) result += chars[offset + i];
    return result;
}

static void replace_all(std::string& value, const std::string& from, const std::string& to) {
    size_t offset = 0;
    while ((offset = value.find(from, offset)) != std::string::npos) {
        value.replace(offset, from.size(), to);
        offset += to.size();
    }
}

static void write_u16(FILE* file, uint16_t value) {
    const uint8_t bytes[] = {static_cast<uint8_t>(value), static_cast<uint8_t>(value >> 8)};
    std::fwrite(bytes, 1, sizeof(bytes), file);
}

static void write_u32(FILE* file, uint32_t value) {
    const uint8_t bytes[] = {static_cast<uint8_t>(value), static_cast<uint8_t>(value >> 8),
                             static_cast<uint8_t>(value >> 16), static_cast<uint8_t>(value >> 24)};
    std::fwrite(bytes, 1, sizeof(bytes), file);
}

static void write_wav(const std::string& path, const std::vector<float>& samples) {
    if (samples.empty()) throw std::runtime_error("speech synthesis returned no samples");
    if (samples.size() > (UINT32_MAX - 36) / 2) throw std::runtime_error("audio output is too large");
    FILE* file = std::fopen(path.c_str(), "wb");
    if (file == nullptr) throw std::runtime_error("could not create WAV output");
    const uint32_t data_size = static_cast<uint32_t>(samples.size() * 2);
    std::fwrite("RIFF", 1, 4, file); write_u32(file, 36 + data_size);
    std::fwrite("WAVEfmt ", 1, 8, file); write_u32(file, 16);
    write_u16(file, 1); write_u16(file, 1); write_u32(file, kSampleRate);
    write_u32(file, kSampleRate * 2); write_u16(file, 2); write_u16(file, 16);
    std::fwrite("data", 1, 4, file); write_u32(file, data_size);
    for (float sample : samples) {
        if (!std::isfinite(sample)) sample = 0.0f;
        const int16_t pcm = static_cast<int16_t>(
            std::lrint(std::clamp(sample, -1.0f, 1.0f) * 32767.0f));
        write_u16(file, static_cast<uint16_t>(pcm));
    }
    const bool write_failed = std::ferror(file) != 0;
    const bool close_failed = std::fclose(file) != 0;
    const bool failed = write_failed || close_failed;
    if (failed) {
        std::remove(path.c_str());
        throw std::runtime_error("could not write WAV output");
    }
}

static std::vector<uint8_t> read_binary(const std::string& path, const char* label) {
    std::ifstream file(path, std::ios::binary | std::ios::ate);
    if (!file) throw std::runtime_error(std::string("could not open ") + label);
    const auto size = file.tellg();
    if (size <= 0 || static_cast<uint64_t>(size) > UINT32_MAX) {
        throw std::runtime_error(std::string(label) + " has an invalid size");
    }
    file.seekg(0);
    std::vector<uint8_t> data(static_cast<size_t>(size));
    if (!file.read(reinterpret_cast<char*>(data.data()), size)) {
        throw std::runtime_error(std::string("could not read ") + label);
    }
    return data;
}

static void check_ort(const OrtApi* api, OrtStatus* status, const char* operation) {
    if (status == nullptr) return;
    const std::string detail = api->GetErrorMessage(status);
    api->ReleaseStatus(status);
    throw std::runtime_error(std::string(operation) + " failed: " + detail);
}

static rknn_context load_rknn(
    const std::string& path, int32_t core_mask, uint32_t expected_inputs,
    uint32_t expected_outputs, const char* label) {
    auto model = read_binary(path, label);
    rknn_context context = 0;
    int result = rknn_init(&context, model.data(), static_cast<uint32_t>(model.size()), 0, nullptr);
    if (result != RKNN_SUCC) {
        throw std::runtime_error(std::string(label) + " initialization failed: " + std::to_string(result));
    }
    try {
        rknn_input_output_num count{};
        result = rknn_query(context, RKNN_QUERY_IN_OUT_NUM, &count, sizeof(count));
        if (result != RKNN_SUCC || count.n_input != expected_inputs || count.n_output != expected_outputs) {
            throw std::runtime_error(std::string(label) + " input/output signature is incompatible");
        }
        result = rknn_set_core_mask(context, static_cast<rknn_core_mask>(core_mask));
        if (result != RKNN_SUCC) {
            throw std::runtime_error(std::string("setting ") + label + " core mask failed: " + std::to_string(result));
        }
    } catch (...) {
        rknn_destroy(context);
        throw;
    }
    return context;
}

static void fft_inverse(std::vector<std::complex<float>>& values) {
    const size_t count = values.size();
    for (size_t i = 1, j = 0; i < count; ++i) {
        size_t bit = count >> 1;
        for (; j & bit; bit >>= 1) j ^= bit;
        j ^= bit;
        if (i < j) std::swap(values[i], values[j]);
    }
    constexpr float kPi = 3.14159265358979323846f;
    for (size_t length = 2; length <= count; length <<= 1) {
        const float angle = 2.0f * kPi / static_cast<float>(length);
        const std::complex<float> root(std::cos(angle), std::sin(angle));
        for (size_t start = 0; start < count; start += length) {
            std::complex<float> weight(1.0f, 0.0f);
            for (size_t index = 0; index < length / 2; ++index) {
                const auto even = values[start + index];
                const auto odd = values[start + index + length / 2] * weight;
                values[start + index] = even + odd;
                values[start + index + length / 2] = even - odd;
                weight *= root;
            }
        }
    }
    for (auto& value : values) value /= static_cast<float>(count);
}

static std::vector<float> istft(
    const float* magnitude, const float* real_direction, const float* imag_direction,
    int frames) {
    if (frames <= 0 || frames > kVocosFrames) throw std::runtime_error("invalid Vocos frame count");
    const size_t full_size = static_cast<size_t>(frames - 1) * kHopLength + kFftSize;
    std::vector<float> audio(full_size, 0.0f);
    std::vector<float> denominator(full_size, 0.0f);
    std::array<float, kFftSize> window{};
    constexpr float kPi = 3.14159265358979323846f;
    for (int i = 0; i < kFftSize; ++i) {
        window[i] = 0.5f - 0.5f * std::cos(2.0f * kPi * i / static_cast<float>(kFftSize - 1));
    }
    std::vector<std::complex<float>> spectrum(kFftSize);
    for (int frame = 0; frame < frames; ++frame) {
        for (int bin = 0; bin < kFftBins; ++bin) {
            const size_t index = static_cast<size_t>(bin) * kVocosFrames + frame;
            const float mag = magnitude[index];
            spectrum[bin] = {mag * real_direction[index], mag * imag_direction[index]};
        }
        for (int bin = 1; bin < kFftBins - 1; ++bin) spectrum[kFftSize - bin] = std::conj(spectrum[bin]);
        fft_inverse(spectrum);
        const size_t start = static_cast<size_t>(frame) * kHopLength;
        for (int sample = 0; sample < kFftSize; ++sample) {
            const float weight = window[sample];
            audio[start + sample] += spectrum[sample].real() * weight;
            denominator[start + sample] += weight * weight;
        }
    }
    for (size_t i = 0; i < audio.size(); ++i) audio[i] /= std::max(denominator[i], 1.0e-8f);
    audio.resize(static_cast<size_t>(frames) * kHopLength);
    return audio;
}

}  // namespace

struct rkserve_matcha_tts_engine {
    rknn_context matcha = 0;
    rknn_context vocos = 0;
    void* ort_library = nullptr;
    const OrtApi* ort = nullptr;
    OrtEnv* ort_environment = nullptr;
    OrtSessionOptions* ort_session_options = nullptr;
    OrtSession* duration_session = nullptr;
    OrtMemoryInfo* ort_memory = nullptr;
    std::unordered_map<std::string, std::vector<int64_t>> lexicon;
    std::unordered_map<std::string, int64_t> tokens;
    size_t max_lexicon_chars = 1;
    std::string espeak_path;
    std::string espeak_data_parent;

    ~rkserve_matcha_tts_engine() {
        if (matcha != 0) rknn_destroy(matcha);
        if (vocos != 0) rknn_destroy(vocos);
        if (ort != nullptr) {
            if (duration_session != nullptr) ort->ReleaseSession(duration_session);
            if (ort_session_options != nullptr) ort->ReleaseSessionOptions(ort_session_options);
            if (ort_memory != nullptr) ort->ReleaseMemoryInfo(ort_memory);
            if (ort_environment != nullptr) ort->ReleaseEnv(ort_environment);
        }
        if (ort_library != nullptr) dlclose(ort_library);
    }

    void load_duration_model(const std::string& library_path, const std::string& model_path) {
        ort_library = dlopen(library_path.c_str(), RTLD_NOW | RTLD_LOCAL);
        if (ort_library == nullptr) {
            throw std::runtime_error(std::string("could not load ONNX Runtime: ") + dlerror());
        }
        dlerror();
        const auto get_api_base = reinterpret_cast<decltype(&OrtGetApiBase)>(
            dlsym(ort_library, "OrtGetApiBase"));
        const char* symbol_error = dlerror();
        if (symbol_error != nullptr || get_api_base == nullptr) {
            throw std::runtime_error(std::string("could not resolve ONNX Runtime API: ") +
                                     (symbol_error == nullptr ? "unknown error" : symbol_error));
        }
        const OrtApiBase* api_base = get_api_base();
        ort = api_base == nullptr ? nullptr : api_base->GetApi(ORT_API_VERSION);
        if (ort == nullptr) throw std::runtime_error("ONNX Runtime API version is incompatible");

        check_ort(ort, ort->CreateEnv(ORT_LOGGING_LEVEL_WARNING, "rkserve-matcha-tts",
                                      &ort_environment), "creating ONNX Runtime environment");
        check_ort(ort, ort->CreateSessionOptions(&ort_session_options),
                  "creating ONNX Runtime session options");
        check_ort(ort, ort->SetIntraOpNumThreads(ort_session_options, 4),
                  "configuring ONNX Runtime intra-op threads");
        check_ort(ort, ort->SetInterOpNumThreads(ort_session_options, 1),
                  "configuring ONNX Runtime inter-op threads");
        check_ort(ort, ort->SetSessionGraphOptimizationLevel(
                           ort_session_options, ORT_ENABLE_ALL),
                  "configuring ONNX Runtime optimizations");
        check_ort(ort, ort->CreateSession(ort_environment, model_path.c_str(),
                                          ort_session_options, &duration_session),
                  "loading Matcha duration model");
        check_ort(ort, ort->CreateCpuMemoryInfo(OrtArenaAllocator, OrtMemTypeDefault,
                                                &ort_memory),
                  "creating ONNX Runtime memory descriptor");
    }

    int run_duration(const std::vector<int64_t>& token_ids, float speed) const {
        std::array<int64_t, kSeqLength> input_ids{};
        std::copy(token_ids.begin(), token_ids.end(), input_ids.begin());
        int64_t input_length = static_cast<int64_t>(token_ids.size());
        float length_scale = 1.0f / speed;
        const int64_t ids_shape[] = {1, kSeqLength};
        const int64_t scalar_shape[] = {1};
        OrtValue* values[3]{};
        OrtValue* output = nullptr;
        try {
            check_ort(ort, ort->CreateTensorWithDataAsOrtValue(
                               ort_memory, input_ids.data(), sizeof(input_ids), ids_shape, 2,
                               ONNX_TENSOR_ELEMENT_DATA_TYPE_INT64, &values[0]),
                      "creating duration token tensor");
            check_ort(ort, ort->CreateTensorWithDataAsOrtValue(
                               ort_memory, &input_length, sizeof(input_length), scalar_shape, 1,
                               ONNX_TENSOR_ELEMENT_DATA_TYPE_INT64, &values[1]),
                      "creating duration length tensor");
            check_ort(ort, ort->CreateTensorWithDataAsOrtValue(
                               ort_memory, &length_scale, sizeof(length_scale), scalar_shape, 1,
                               ONNX_TENSOR_ELEMENT_DATA_TYPE_FLOAT, &values[2]),
                      "creating duration speed tensor");
            const char* input_names[] = {"x", "x_length", "length_scale"};
            const char* output_names[] = {"/ReduceMax_output_0"};
            const OrtValue* input_values[] = {values[0], values[1], values[2]};
            check_ort(ort, ort->Run(duration_session, nullptr, input_names,
                                    input_values, 3,
                                    output_names, 1, &output),
                      "running Matcha duration model");
            void* output_data = nullptr;
            check_ort(ort, ort->GetTensorMutableData(output, &output_data),
                      "reading Matcha duration output");
            const int64_t frames = *static_cast<const int64_t*>(output_data);
            ort->ReleaseValue(output);
            output = nullptr;
            for (OrtValue*& value : values) {
                ort->ReleaseValue(value);
                value = nullptr;
            }
            if (frames <= 0 || frames > kMatchaFrames) {
                throw std::runtime_error("Matcha duration is outside the supported frame bucket");
            }
            return static_cast<int>(frames);
        } catch (...) {
            if (output != nullptr) ort->ReleaseValue(output);
            for (OrtValue* value : values) if (value != nullptr) ort->ReleaseValue(value);
            throw;
        }
    }

    void load_tokens(const std::string& path) {
        std::ifstream file(path);
        if (!file) throw std::runtime_error("could not open tokens.txt");
        for (std::string line; std::getline(file, line);) {
            const auto fields = split_ws(line);
            if (fields.size() == 1) tokens.emplace(" ", std::stoll(fields[0]));
            else if (fields.size() == 2) tokens.emplace(fields[0], std::stoll(fields[1]));
        }
        if (tokens.size() < 2000 || !tokens.count(" ")) {
            throw std::runtime_error("tokens.txt is incomplete");
        }
    }

    void load_lexicon(const std::string& path) {
        std::ifstream file(path);
        if (!file) throw std::runtime_error("could not open lexicon.txt");
        for (std::string line; std::getline(file, line);) {
            const auto fields = split_ws(line);
            if (fields.size() < 2) continue;
            std::vector<int64_t> ids;
            bool valid = true;
            for (size_t i = 1; i < fields.size(); ++i) {
                const auto found = tokens.find(fields[i]);
                if (found == tokens.end()) { valid = false; break; }
                ids.push_back(found->second);
            }
            if (!valid || ids.empty()) continue;
            const std::string word = lower_ascii(fields[0]);
            max_lexicon_chars = std::max(max_lexicon_chars, utf8_chars(word).size());
            lexicon.emplace(word, std::move(ids));
        }
        if (lexicon.size() < 60000) throw std::runtime_error("lexicon.txt is incomplete");
    }

    std::vector<int64_t> phonemize_english(const std::string& text) const {
        int pipes[2];
        if (pipe(pipes) != 0) throw std::runtime_error("could not create eSpeak pipe");
        const std::string data_argument = "--path=" + espeak_data_parent;
        const pid_t child = fork();
        if (child < 0) {
            close(pipes[0]); close(pipes[1]);
            throw std::runtime_error("could not start eSpeak NG");
        }
        if (child == 0) {
            dup2(pipes[1], STDOUT_FILENO);
            close(pipes[0]); close(pipes[1]);
            execl(espeak_path.c_str(), espeak_path.c_str(), data_argument.c_str(), "--ipa", "-q",
                  "-v", "en-us", "--", text.c_str(), static_cast<char*>(nullptr));
            _exit(127);
        }
        close(pipes[1]);
        std::string ipa;
        std::array<char, 4096> buffer{};
        while (true) {
            const ssize_t count = read(pipes[0], buffer.data(), buffer.size());
            if (count == 0) break;
            if (count < 0) { close(pipes[0]); waitpid(child, nullptr, 0); throw std::runtime_error("could not read eSpeak output"); }
            ipa.append(buffer.data(), static_cast<size_t>(count));
            if (ipa.size() > 1024 * 1024) { close(pipes[0]); waitpid(child, nullptr, 0); throw std::runtime_error("eSpeak output is too large"); }
        }
        close(pipes[0]);
        int status = 0;
        if (waitpid(child, &status, 0) < 0 || !WIFEXITED(status) || WEXITSTATUS(status) != 0) {
            throw std::runtime_error("eSpeak NG phonemization failed");
        }
        static const std::pair<const char*, const char*> replacements[] = {
            {"ɝ", "ɜɹ"}, {"ɚ", "əɹ"}, {"eɪ", "A"}, {"aɪ", "I"}, {"ɔɪ", "Y"},
            {"oʊ", "O"}, {"əʊ", "O"}, {"aʊ", "W"}, {"tʃ", "ʧ"}, {"dʒ", "ʤ"},
            {"ː", ""}, {"g", "ɡ"}, {"r", "ɹ"}, {"e", "ɛ"},
        };
        for (const auto& [from, to] : replacements) replace_all(ipa, from, to);
        std::vector<int64_t> ids;
        bool last_blank = true;
        for (const std::string& symbol : utf8_chars(ipa)) {
            if (whitespace(symbol)) {
                if (!last_blank && tokens.count(" ")) { ids.push_back(tokens.at(" ")); last_blank = true; }
                continue;
            }
            const auto found = tokens.find(symbol);
            if (found != tokens.end()) { ids.push_back(found->second); last_blank = false; }
        }
        while (!ids.empty() && ids.back() == tokens.at(" ")) ids.pop_back();
        if (ids.empty()) throw std::runtime_error("English text produced no supported phonemes");
        return ids;
    }

    void append_ids(std::vector<std::vector<int64_t>>& sentences,
                    std::vector<int64_t>& current, const std::vector<int64_t>& ids) const {
        for (int64_t id : ids) {
            if (current.size() == kMaxTokens) {
                sentences.push_back(std::move(current));
                current.clear();
            }
            current.push_back(id);
        }
    }

    std::vector<std::vector<int64_t>> encode_text(const std::string& text) const {
        auto chars = utf8_chars(text);
        for (std::string& value : chars) value = normalize_punctuation(value);
        std::vector<std::vector<int64_t>> sentences;
        std::vector<int64_t> current;
        for (size_t index = 0; index < chars.size();) {
            const std::string& value = chars[index];
            if (ascii_letter(value)) {
                std::string english;
                while (index < chars.size()) {
                    if (ascii_word_part(chars[index]) || whitespace(chars[index])) {
                        english += chars[index++];
                    } else {
                        break;
                    }
                }
                append_ids(sentences, current, phonemize_english(english));
                continue;
            }
            if (ascii_digit(value)) {
                static const char* chinese_digits[] = {"零", "一", "二", "三", "四", "五", "六", "七", "八", "九"};
                std::vector<int64_t> number_ids;
                while (index < chars.size() && ascii_digit(chars[index])) {
                    const auto found = lexicon.find(chinese_digits[chars[index][0] - '0']);
                    if (found != lexicon.end()) number_ids.insert(number_ids.end(), found->second.begin(), found->second.end());
                    ++index;
                }
                append_ids(sentences, current, number_ids);
                continue;
            }
            if (whitespace(value)) { ++index; continue; }
            if (tokens.count(value)) {
                append_ids(sentences, current, {tokens.at(value)});
                ++index;
                if (sentence_boundary(value) && !current.empty()) {
                    sentences.push_back(std::move(current));
                    current.clear();
                }
                continue;
            }
            bool found_word = false;
            const size_t available = chars.size() - index;
            for (size_t length = std::min(max_lexicon_chars, available); length > 0; --length) {
                const auto found = lexicon.find(lower_ascii(join_chars(chars, index, length)));
                if (found == lexicon.end()) continue;
                append_ids(sentences, current, found->second);
                index += length;
                found_word = true;
                break;
            }
            if (!found_word) ++index;
        }
        if (!current.empty()) sentences.push_back(std::move(current));
        sentences.erase(
            std::remove_if(sentences.begin(), sentences.end(), [](const auto& sentence) { return sentence.empty(); }),
            sentences.end());
        if (sentences.empty()) throw std::runtime_error("speech input has no pronounceable text");
        return sentences;
    }

    std::pair<std::vector<float>, int> run_matcha(
        const std::vector<int64_t>& token_ids, float speed, float noise_scale,
        uint64_t& preprocess_us, uint64_t& inference_us) {
        auto duration_begin = Clock::now();
        const int frames = run_duration(token_ids, speed);
        preprocess_us += elapsed_us(duration_begin);
        std::array<int64_t, kSeqLength> input_ids{};
        std::copy(token_ids.begin(), token_ids.end(), input_ids.begin());
        int64_t input_length = static_cast<int64_t>(token_ids.size());
        float length_scale = 1.0f / speed;
        rknn_input inputs[4]{};
        inputs[0].index = 0; inputs[0].buf = input_ids.data(); inputs[0].size = sizeof(input_ids);
        inputs[0].type = RKNN_TENSOR_INT64; inputs[0].fmt = RKNN_TENSOR_UNDEFINED;
        inputs[1].index = 1; inputs[1].buf = &input_length; inputs[1].size = sizeof(input_length);
        inputs[1].type = RKNN_TENSOR_INT64; inputs[1].fmt = RKNN_TENSOR_UNDEFINED;
        inputs[2].index = 2; inputs[2].buf = &noise_scale; inputs[2].size = sizeof(noise_scale);
        inputs[2].type = RKNN_TENSOR_FLOAT32; inputs[2].fmt = RKNN_TENSOR_UNDEFINED;
        inputs[3].index = 3; inputs[3].buf = &length_scale; inputs[3].size = sizeof(length_scale);
        inputs[3].type = RKNN_TENSOR_FLOAT32; inputs[3].fmt = RKNN_TENSOR_UNDEFINED;
        int result = rknn_inputs_set(matcha, 4, inputs);
        if (result != RKNN_SUCC) throw std::runtime_error("Matcha input failed: " + std::to_string(result));
        const auto begin = Clock::now();
        result = rknn_run(matcha, nullptr);
        inference_us += elapsed_us(begin);
        if (result != RKNN_SUCC) throw std::runtime_error("Matcha inference failed: " + std::to_string(result));
        rknn_output outputs[2]{};
        outputs[0].index = 0; outputs[0].want_float = 1;
        outputs[1].index = 1; outputs[1].want_float = 0;
        result = rknn_outputs_get(matcha, 2, outputs, nullptr);
        if (result != RKNN_SUCC) throw std::runtime_error("Matcha output failed: " + std::to_string(result));
        try {
            if (outputs[0].size < kMelChannels * kMatchaFrames * sizeof(float) || outputs[1].size < sizeof(int64_t)) {
                throw std::runtime_error("Matcha returned a short output");
            }
            const float* data = static_cast<const float*>(outputs[0].buf);
            std::vector<float> mel(data, data + kMelChannels * kMatchaFrames);
            rknn_outputs_release(matcha, 2, outputs);
            smooth_mel(mel, frames);
            return {std::move(mel), frames};
        } catch (...) {
            rknn_outputs_release(matcha, 2, outputs);
            throw;
        }
    }

    static void smooth_mel(std::vector<float>& mel, int frames) {
        if (frames < 5) return;
        std::vector<float> energy(frames, 0.0f);
        for (int frame = 0; frame < frames; ++frame) {
            for (int channel = 0; channel < kMelChannels; ++channel) {
                const float value = mel[static_cast<size_t>(channel) * kMatchaFrames + frame];
                energy[frame] += value * value;
            }
            energy[frame] /= kMelChannels;
        }
        const auto original = mel;
        for (int frame = 0; frame < frames; ++frame) {
            std::vector<float> neighborhood;
            for (int i = std::max(0, frame - 2); i <= std::min(frames - 1, frame + 2); ++i) {
                neighborhood.push_back(energy[i]);
            }
            std::sort(neighborhood.begin(), neighborhood.end());
            const float median = neighborhood[neighborhood.size() / 2];
            const float ratio = energy[frame] / std::max(median, 1.0e-8f);
            if (ratio >= 0.5f && ratio <= 1.8f) continue;
            const int left = std::max(0, frame - 1), right = std::min(frames - 1, frame + 1);
            for (int channel = 0; channel < kMelChannels; ++channel) {
                const size_t base = static_cast<size_t>(channel) * kMatchaFrames;
                mel[base + frame] = 0.5f * (original[base + left] + original[base + right]);
            }
        }
    }

    std::vector<float> run_vocos(
        const std::vector<float>& mel, int frames, uint64_t& inference_us) {
        std::vector<float> padded(kMelChannels * kVocosFrames, 0.0f);
        for (int channel = 0; channel < kMelChannels; ++channel) {
            std::copy_n(mel.data() + static_cast<size_t>(channel) * kMatchaFrames,
                        kMatchaFrames, padded.data() + static_cast<size_t>(channel) * kVocosFrames);
        }
        rknn_input input{};
        input.index = 0; input.buf = padded.data(); input.size = padded.size() * sizeof(float);
        input.type = RKNN_TENSOR_FLOAT32; input.fmt = RKNN_TENSOR_NCHW;
        int result = rknn_inputs_set(vocos, 1, &input);
        if (result != RKNN_SUCC) throw std::runtime_error("Vocos input failed: " + std::to_string(result));
        const auto begin = Clock::now();
        result = rknn_run(vocos, nullptr);
        inference_us += elapsed_us(begin);
        if (result != RKNN_SUCC) throw std::runtime_error("Vocos inference failed: " + std::to_string(result));
        rknn_output outputs[3]{};
        for (int i = 0; i < 3; ++i) { outputs[i].index = i; outputs[i].want_float = 1; }
        result = rknn_outputs_get(vocos, 3, outputs, nullptr);
        if (result != RKNN_SUCC) throw std::runtime_error("Vocos output failed: " + std::to_string(result));
        try {
            const size_t expected = kFftBins * kVocosFrames * sizeof(float);
            if (outputs[0].size < expected || outputs[1].size < expected || outputs[2].size < expected) {
                throw std::runtime_error("Vocos returned a short output");
            }
            auto audio = istft(
                static_cast<const float*>(outputs[0].buf),
                static_cast<const float*>(outputs[1].buf),
                static_cast<const float*>(outputs[2].buf), frames);
            rknn_outputs_release(vocos, 3, outputs);
            return audio;
        } catch (...) {
            rknn_outputs_release(vocos, 3, outputs);
            throw;
        }
    }
};

extern "C" rkserve_matcha_tts_engine* rkserve_matcha_tts_create(
    const char* matcha_path, const char* vocos_path, const char* lexicon_path,
    const char* tokens_path, const char* espeak_path, const char* espeak_data_parent,
    const char* ort_library_path, const char* duration_model_path,
    int32_t core_mask, char* error, size_t error_capacity) {
    try {
        if (!matcha_path || !vocos_path || !lexicon_path || !tokens_path || !espeak_path ||
            !espeak_data_parent || !ort_library_path || !duration_model_path) {
            throw std::runtime_error("Matcha TTS asset paths are required");
        }
        auto engine = std::make_unique<rkserve_matcha_tts_engine>();
        engine->espeak_path = espeak_path;
        engine->espeak_data_parent = espeak_data_parent;
        engine->load_tokens(tokens_path);
        engine->load_lexicon(lexicon_path);
        engine->load_duration_model(ort_library_path, duration_model_path);
        engine->matcha = load_rknn(matcha_path, core_mask, 4, 2, "Matcha RKNN model");
        engine->vocos = load_rknn(vocos_path, core_mask, 1, 3, "Vocos RKNN model");
        return engine.release();
    } catch (const std::exception& exception) {
        write_error(error, error_capacity, exception.what());
        return nullptr;
    }
}

extern "C" void rkserve_matcha_tts_destroy(rkserve_matcha_tts_engine* engine) {
    delete engine;
}

extern "C" int rkserve_matcha_tts_synthesize(
    rkserve_matcha_tts_engine* engine, const char* text, const char* output_path,
    float speed, float noise_scale, int32_t* output_samples,
    uint64_t* preprocess_us, uint64_t* inference_us, uint64_t* postprocess_us,
    char* error, size_t error_capacity) {
    try {
        if (!engine || !text || !output_path || !output_samples || !preprocess_us || !inference_us || !postprocess_us) {
            throw std::runtime_error("invalid Matcha TTS request");
        }
        if (!std::isfinite(speed) || speed < 0.7f || speed > 1.4f ||
            !std::isfinite(noise_scale) || noise_scale < 0.3f || noise_scale > 1.0f) {
            throw std::runtime_error("invalid Matcha TTS generation parameters");
        }
        *preprocess_us = *inference_us = *postprocess_us = 0;
        auto begin = Clock::now();
        const auto sentences = engine->encode_text(text);
        *preprocess_us = elapsed_us(begin);
        std::vector<float> audio;
        for (const auto& sentence : sentences) {
            auto [mel, frames] = engine->run_matcha(
                sentence, speed, noise_scale, *preprocess_us, *inference_us);
            begin = Clock::now();
            const uint64_t inference_before_vocos = *inference_us;
            auto segment = engine->run_vocos(mel, frames, *inference_us);
            const uint64_t vocos_total_us = elapsed_us(begin);
            const uint64_t vocos_inference_us = *inference_us - inference_before_vocos;
            *postprocess_us += vocos_total_us > vocos_inference_us
                ? vocos_total_us - vocos_inference_us
                : 0;
            audio.insert(audio.end(), segment.begin(), segment.end());
        }
        begin = Clock::now();
        float peak = 0.0f;
        for (float sample : audio) if (std::isfinite(sample)) peak = std::max(peak, std::abs(sample));
        if (peak <= 1.0e-8f) throw std::runtime_error("Matcha TTS returned silent audio");
        const float gain = 0.95f / peak;
        for (float& sample : audio) sample = std::isfinite(sample) ? sample * gain : 0.0f;
        write_wav(output_path, audio);
        *postprocess_us += elapsed_us(begin);
        if (audio.size() > static_cast<size_t>(INT32_MAX)) throw std::runtime_error("audio output is too long");
        *output_samples = static_cast<int32_t>(audio.size());
        return 0;
    } catch (const std::exception& exception) {
        write_error(error, error_capacity, exception.what());
        return -1;
    }
}

extern "C" int rkserve_matcha_tts_versions(
    rkserve_matcha_tts_engine* engine, char* runtime, size_t runtime_capacity,
    char* driver, size_t driver_capacity) {
    if (!engine || engine->matcha == 0) return -1;
    rknn_sdk_version version{};
    if (rknn_query(engine->matcha, RKNN_QUERY_SDK_VERSION, &version, sizeof(version)) != RKNN_SUCC) return -1;
    if (runtime && runtime_capacity) std::snprintf(runtime, runtime_capacity, "%s", version.api_version);
    if (driver && driver_capacity) std::snprintf(driver, driver_capacity, "%s", version.drv_version);
    return 0;
}
