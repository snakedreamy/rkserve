#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <limits>
#include <new>
#include <string>
#include <vector>

#include "Float16.h"
#include "im2d.h"
#include "rga.h"
#include "rknn_api.h"

namespace {

constexpr int kInputWidth = 640;
constexpr int kInputHeight = 640;

struct NativeDetection {
  int32_t class_id;
  float confidence;
  float left;
  float top;
  float right;
  float bottom;
};

struct NativeTiming {
  uint64_t preprocess_us;
  uint64_t inference_us;
  uint64_t postprocess_us;
  uint8_t rga_used;
};

struct Candidate {
  int32_t class_id;
  float confidence;
  float left;
  float top;
  float right;
  float bottom;
};

struct LetterboxInfo {
  float scale;
  int left;
  int top;
};

struct Engine {
  rknn_context context = 0;
  rknn_tensor_attr output_attr{};
  int class_count = 0;
  int channel_count = 0;
  int anchor_count = 0;
  bool channel_major = true;
  std::string api_version;
  std::string driver_version;
};

using Clock = std::chrono::steady_clock;

uint64_t elapsed_us(Clock::time_point start, Clock::time_point end) {
  return static_cast<uint64_t>(
      std::chrono::duration_cast<std::chrono::microseconds>(end - start).count());
}

void set_error(char* error, size_t capacity, const std::string& message) {
  if (error == nullptr || capacity == 0) {
    return;
  }
  std::snprintf(error, capacity, "%s", message.c_str());
}

bool query_tensor(rknn_context context, rknn_query_cmd command,
                  rknn_tensor_attr* attr, std::string* error) {
  std::memset(attr, 0, sizeof(*attr));
  attr->index = 0;
  const int result = rknn_query(context, command, attr, sizeof(*attr));
  if (result != RKNN_SUCC) {
    *error = "rknn_query tensor failed: " + std::to_string(result);
    return false;
  }
  return true;
}

bool validate_model(Engine* engine, std::string* error) {
  rknn_input_output_num io{};
  int result = rknn_query(engine->context, RKNN_QUERY_IN_OUT_NUM, &io, sizeof(io));
  if (result != RKNN_SUCC) {
    *error = "rknn_query input/output count failed: " + std::to_string(result);
    return false;
  }
  if (io.n_input != 1 || io.n_output != 1) {
    *error = "YOLO26 requires exactly one input and one output, got " +
             std::to_string(io.n_input) + " input(s) and " +
             std::to_string(io.n_output) + " output(s)";
    return false;
  }

  rknn_tensor_attr input{};
  if (!query_tensor(engine->context, RKNN_QUERY_INPUT_ATTR, &input, error) ||
      !query_tensor(engine->context, RKNN_QUERY_OUTPUT_ATTR, &engine->output_attr, error)) {
    return false;
  }
  if (input.n_elems != static_cast<uint32_t>(kInputWidth * kInputHeight * 3)) {
    *error = "unexpected YOLO26 input element count: " + std::to_string(input.n_elems);
    return false;
  }
  if (engine->channel_count <= 4 ||
      engine->output_attr.n_elems % engine->channel_count != 0) {
    *error = "YOLO26 output element count is incompatible with metadata class count: " +
             std::to_string(engine->output_attr.n_elems) + " elements for " +
             std::to_string(engine->class_count) + " classes";
    return false;
  }
  engine->anchor_count = static_cast<int>(engine->output_attr.n_elems) /
                         engine->channel_count;
  if (engine->output_attr.type != RKNN_TENSOR_FLOAT16 &&
      engine->output_attr.type != RKNN_TENSOR_FLOAT32) {
    *error = std::string("YOLO26 output must be FP16 or FP32, got ") +
             get_type_string(engine->output_attr.type);
    return false;
  }

  int channel_dimension = -1;
  int anchor_dimension = -1;
  for (uint32_t index = 0; index < engine->output_attr.n_dims; ++index) {
    if (engine->output_attr.dims[index] == static_cast<uint32_t>(engine->channel_count)) {
      channel_dimension = static_cast<int>(index);
    }
    if (engine->output_attr.dims[index] == static_cast<uint32_t>(engine->anchor_count)) {
      anchor_dimension = static_cast<int>(index);
    }
  }
  if (channel_dimension < 0 || anchor_dimension < 0) {
    *error = "YOLO26 output dimensions do not match " +
             std::to_string(engine->channel_count) + " channels and " +
             std::to_string(engine->anchor_count) + " anchors";
    return false;
  }
  engine->channel_major = channel_dimension < anchor_dimension;
  return true;
}

void resize_bilinear_rgb(const uint8_t* source, int source_width, int source_height,
                         uint8_t* destination, int destination_stride,
                         int destination_left, int destination_top,
                         int resized_width, int resized_height) {
  const float scale_x = static_cast<float>(source_width) / resized_width;
  const float scale_y = static_cast<float>(source_height) / resized_height;
  for (int y = 0; y < resized_height; ++y) {
    const float source_y = (y + 0.5f) * scale_y - 0.5f;
    const int y0 = std::max(0, static_cast<int>(std::floor(source_y)));
    const int y1 = std::min(source_height - 1, y0 + 1);
    const float fy = std::max(0.0f, source_y - y0);
    uint8_t* row = destination +
                   ((destination_top + y) * destination_stride + destination_left) * 3;
    for (int x = 0; x < resized_width; ++x) {
      const float source_x = (x + 0.5f) * scale_x - 0.5f;
      const int x0 = std::max(0, static_cast<int>(std::floor(source_x)));
      const int x1 = std::min(source_width - 1, x0 + 1);
      const float fx = std::max(0.0f, source_x - x0);
      for (int channel = 0; channel < 3; ++channel) {
        const float top = source[(y0 * source_width + x0) * 3 + channel] * (1.0f - fx) +
                          source[(y0 * source_width + x1) * 3 + channel] * fx;
        const float bottom = source[(y1 * source_width + x0) * 3 + channel] * (1.0f - fx) +
                             source[(y1 * source_width + x1) * 3 + channel] * fx;
        row[x * 3 + channel] = static_cast<uint8_t>(
            std::clamp(std::round(top * (1.0f - fy) + bottom * fy), 0.0f, 255.0f));
      }
    }
  }
}

bool letterbox_rgb(const uint8_t* source, int width, int height,
                   std::vector<uint8_t>* destination, LetterboxInfo* info,
                   bool* rga_used, std::string* error) {
  if (source == nullptr || width <= 0 || height <= 0) {
    *error = "invalid RGB image dimensions";
    return false;
  }
  info->scale = std::min(static_cast<float>(kInputWidth) / width,
                         static_cast<float>(kInputHeight) / height);
  const int resized_width = std::clamp(
      static_cast<int>(std::round(width * info->scale)), 1, kInputWidth);
  const int resized_height = std::clamp(
      static_cast<int>(std::round(height * info->scale)), 1, kInputHeight);
  info->left = (kInputWidth - resized_width) / 2;
  info->top = (kInputHeight - resized_height) / 2;

  destination->assign(kInputWidth * kInputHeight * 3, 114);
  const int source_stride = (width + 15) & ~15;
  std::vector<uint8_t> aligned_source;
  const uint8_t* rga_source = source;
  if (source_stride != width) {
    aligned_source.resize(static_cast<size_t>(source_stride) * height * 3);
    for (int y = 0; y < height; ++y) {
      std::memcpy(aligned_source.data() + static_cast<size_t>(y) * source_stride * 3,
                  source + static_cast<size_t>(y) * width * 3,
                  static_cast<size_t>(width) * 3);
    }
    rga_source = aligned_source.data();
  }

  rga_buffer_t source_buffer = wrapbuffer_virtualaddr(
      const_cast<uint8_t*>(rga_source), width, height, RK_FORMAT_RGB_888,
      source_stride, height);
  rga_buffer_t destination_buffer = wrapbuffer_virtualaddr(
      destination->data(), kInputWidth, kInputHeight, RK_FORMAT_RGB_888);
  rga_buffer_t empty_buffer{};
  im_rect source_rect{0, 0, width, height};
  im_rect destination_rect{info->left, info->top, resized_width, resized_height};
  im_rect empty_rect{};
  const IM_STATUS rga_result = improcess(source_buffer, destination_buffer, empty_buffer,
                                         source_rect, destination_rect, empty_rect, IM_SYNC);
  *rga_used = rga_result > 0;
  if (rga_result <= 0) {
    resize_bilinear_rgb(source, width, height, destination->data(), kInputWidth,
                        info->left, info->top, resized_width, resized_height);
  }
  return true;
}

float tensor_value(const Engine* engine, const void* data, int channel, int anchor) {
  const size_t index = engine->channel_major
                           ? static_cast<size_t>(channel) * engine->anchor_count + anchor
                           : static_cast<size_t>(anchor) * engine->channel_count + channel;
  if (engine->output_attr.type == RKNN_TENSOR_FLOAT16) {
    const auto* values = static_cast<const uint16_t*>(data);
    return static_cast<float>(rknpu2::float16::fromBits(values[index]));
  }
  return static_cast<const float*>(data)[index];
}

float intersection_over_union(const Candidate& first, const Candidate& second) {
  const float left = std::max(first.left, second.left);
  const float top = std::max(first.top, second.top);
  const float right = std::min(first.right, second.right);
  const float bottom = std::min(first.bottom, second.bottom);
  const float intersection = std::max(0.0f, right - left) *
                             std::max(0.0f, bottom - top);
  const float first_area = std::max(0.0f, first.right - first.left) *
                           std::max(0.0f, first.bottom - first.top);
  const float second_area = std::max(0.0f, second.right - second.left) *
                            std::max(0.0f, second.bottom - second.top);
  const float union_area = first_area + second_area - intersection;
  return union_area > 0.0f ? intersection / union_area : 0.0f;
}

std::vector<Candidate> postprocess(const Engine* engine, const void* output,
                                   const LetterboxInfo& letterbox,
                                   int original_width, int original_height,
                                   float confidence_threshold, float iou_threshold,
                                   int max_detections) {
  std::vector<Candidate> candidates;
  candidates.reserve(512);
  for (int anchor = 0; anchor < engine->anchor_count; ++anchor) {
    int class_id = 0;
    float confidence = tensor_value(engine, output, 4, anchor);
    for (int candidate_class = 1; candidate_class < engine->class_count; ++candidate_class) {
      const float score = tensor_value(engine, output, 4 + candidate_class, anchor);
      if (score > confidence) {
        confidence = score;
        class_id = candidate_class;
      }
    }
    if (!std::isfinite(confidence) || confidence <= confidence_threshold) {
      continue;
    }

    const float center_x = tensor_value(engine, output, 0, anchor);
    const float center_y = tensor_value(engine, output, 1, anchor);
    const float box_width = tensor_value(engine, output, 2, anchor);
    const float box_height = tensor_value(engine, output, 3, anchor);
    if (!std::isfinite(center_x) || !std::isfinite(center_y) ||
        !std::isfinite(box_width) || !std::isfinite(box_height) ||
        box_width <= 0.0f || box_height <= 0.0f) {
      continue;
    }

    Candidate candidate{};
    candidate.class_id = class_id;
    candidate.confidence = confidence;
    candidate.left = std::clamp(
        (center_x - box_width * 0.5f - letterbox.left) / letterbox.scale,
        0.0f, static_cast<float>(original_width));
    candidate.top = std::clamp(
        (center_y - box_height * 0.5f - letterbox.top) / letterbox.scale,
        0.0f, static_cast<float>(original_height));
    candidate.right = std::clamp(
        (center_x + box_width * 0.5f - letterbox.left) / letterbox.scale,
        0.0f, static_cast<float>(original_width));
    candidate.bottom = std::clamp(
        (center_y + box_height * 0.5f - letterbox.top) / letterbox.scale,
        0.0f, static_cast<float>(original_height));
    if (candidate.right > candidate.left && candidate.bottom > candidate.top) {
      candidates.push_back(candidate);
    }
  }

  std::stable_sort(candidates.begin(), candidates.end(),
                   [](const Candidate& first, const Candidate& second) {
                     return first.confidence > second.confidence;
                   });
  std::vector<Candidate> selected;
  selected.reserve(std::min(static_cast<size_t>(max_detections), candidates.size()));
  for (const Candidate& candidate : candidates) {
    bool suppressed = false;
    for (const Candidate& kept : selected) {
      if (candidate.class_id == kept.class_id &&
          intersection_over_union(candidate, kept) > iou_threshold) {
        suppressed = true;
        break;
      }
    }
    if (!suppressed) {
      selected.push_back(candidate);
      if (static_cast<int>(selected.size()) >= max_detections) {
        break;
      }
    }
  }
  return selected;
}

}  // namespace

extern "C" Engine* rkserve_yolo26_create(const char* model_path, int class_count,
                                          int core_mask,
                                          char* error, size_t error_capacity) {
  if (model_path == nullptr || model_path[0] == '\0' ||
      class_count <= 0 || class_count > 4096) {
    set_error(error, error_capacity, "model path or metadata class count is invalid");
    return nullptr;
  }
  auto* engine = new (std::nothrow) Engine();
  if (engine == nullptr) {
    set_error(error, error_capacity, "could not allocate YOLO26 engine");
    return nullptr;
  }
  engine->class_count = class_count;
  engine->channel_count = class_count + 4;

  int result = rknn_init(&engine->context, const_cast<char*>(model_path), 0, 0, nullptr);
  if (result != RKNN_SUCC) {
    set_error(error, error_capacity, "rknn_init failed: " + std::to_string(result));
    delete engine;
    return nullptr;
  }
  result = rknn_set_core_mask(engine->context, static_cast<rknn_core_mask>(core_mask));
  if (result != RKNN_SUCC) {
    set_error(error, error_capacity, "rknn_set_core_mask failed: " + std::to_string(result));
    rknn_destroy(engine->context);
    delete engine;
    return nullptr;
  }

  std::string validation_error;
  if (!validate_model(engine, &validation_error)) {
    set_error(error, error_capacity, validation_error);
    rknn_destroy(engine->context);
    delete engine;
    return nullptr;
  }
  rknn_sdk_version versions{};
  result = rknn_query(engine->context, RKNN_QUERY_SDK_VERSION, &versions, sizeof(versions));
  if (result == RKNN_SUCC) {
    engine->api_version = versions.api_version;
    engine->driver_version = versions.drv_version;
  }
  return engine;
}

extern "C" void rkserve_yolo26_destroy(Engine* engine) {
  if (engine == nullptr) {
    return;
  }
  if (engine->context != 0) {
    rknn_destroy(engine->context);
  }
  delete engine;
}

extern "C" int rkserve_yolo26_versions(Engine* engine, char* api_version,
                                        size_t api_capacity, char* driver_version,
                                        size_t driver_capacity) {
  if (engine == nullptr) {
    return -1;
  }
  set_error(api_version, api_capacity, engine->api_version);
  set_error(driver_version, driver_capacity, engine->driver_version);
  return 0;
}

extern "C" int rkserve_yolo26_infer(
    Engine* engine, const uint8_t* rgb, int width, int height,
    float confidence_threshold, float iou_threshold, int max_detections,
    NativeDetection* detections, int capacity, NativeTiming* timing,
    char* error, size_t error_capacity) {
  if (engine == nullptr || rgb == nullptr || detections == nullptr || timing == nullptr ||
      width <= 0 || height <= 0 || capacity <= 0 || max_detections <= 0 ||
      !std::isfinite(confidence_threshold) || confidence_threshold < 0.0f ||
      confidence_threshold > 1.0f || !std::isfinite(iou_threshold) ||
      iou_threshold < 0.0f || iou_threshold > 1.0f) {
    set_error(error, error_capacity, "invalid YOLO26 inference arguments");
    return -1;
  }

  const auto preprocess_start = Clock::now();
  std::vector<uint8_t> input_data;
  LetterboxInfo letterbox{};
  bool rga_used = false;
  std::string preprocess_error;
  if (!letterbox_rgb(rgb, width, height, &input_data, &letterbox, &rga_used,
                     &preprocess_error)) {
    set_error(error, error_capacity, preprocess_error);
    return -1;
  }
  const auto preprocess_end = Clock::now();

  rknn_input input{};
  input.index = 0;
  input.buf = input_data.data();
  input.size = static_cast<uint32_t>(input_data.size());
  input.pass_through = 0;
  input.type = RKNN_TENSOR_UINT8;
  input.fmt = RKNN_TENSOR_NHWC;

  const auto inference_start = Clock::now();
  int result = rknn_inputs_set(engine->context, 1, &input);
  if (result != RKNN_SUCC) {
    set_error(error, error_capacity, "rknn_inputs_set failed: " + std::to_string(result));
    return -1;
  }
  result = rknn_run(engine->context, nullptr);
  if (result != RKNN_SUCC) {
    set_error(error, error_capacity, "rknn_run failed: " + std::to_string(result));
    return -1;
  }
  rknn_output output{};
  output.index = 0;
  output.want_float = 0;
  output.is_prealloc = 0;
  result = rknn_outputs_get(engine->context, 1, &output, nullptr);
  const auto inference_end = Clock::now();
  if (result != RKNN_SUCC) {
    set_error(error, error_capacity, "rknn_outputs_get failed: " + std::to_string(result));
    return -1;
  }

  const auto postprocess_start = Clock::now();
  const std::vector<Candidate> selected = postprocess(
      engine, output.buf, letterbox, width, height, confidence_threshold,
      iou_threshold, std::min(max_detections, capacity));
  const auto postprocess_end = Clock::now();
  rknn_outputs_release(engine->context, 1, &output);

  for (size_t index = 0; index < selected.size(); ++index) {
    detections[index] = NativeDetection{
        selected[index].class_id, selected[index].confidence,
        selected[index].left, selected[index].top,
        selected[index].right, selected[index].bottom};
  }
  timing->preprocess_us = elapsed_us(preprocess_start, preprocess_end);
  timing->inference_us = elapsed_us(inference_start, inference_end);
  timing->postprocess_us = elapsed_us(postprocess_start, postprocess_end);
  timing->rga_used = rga_used ? 1 : 0;
  return static_cast<int>(selected.size());
}
