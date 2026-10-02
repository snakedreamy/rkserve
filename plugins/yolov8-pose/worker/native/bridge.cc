// SPDX-License-Identifier: Apache-2.0

#include <cstdio>
#include <cstring>
#include <new>

#include "yolov8-pose.h"

struct rkserve_keypoint {
    float x;
    float y;
    float confidence;
};

struct rkserve_pose {
    float confidence;
    int32_t left;
    int32_t top;
    int32_t right;
    int32_t bottom;
    rkserve_keypoint keypoints[17];
};

struct rkserve_yolov8_pose_engine {
    rknn_app_context_t context;
};

static void write_error(char* error, size_t capacity, const char* message) {
    if (error == nullptr || capacity == 0) {
        return;
    }
    std::snprintf(error, capacity, "%s", message);
}

extern "C" rkserve_yolov8_pose_engine* rkserve_yolov8_pose_create(
    const char* model_path,
    int32_t core_mask,
    char* error,
    size_t error_capacity) {
    auto* engine = new (std::nothrow) rkserve_yolov8_pose_engine{};
    if (engine == nullptr) {
        write_error(error, error_capacity, "could not allocate engine");
        return nullptr;
    }
    if (init_post_process() != 0) {
        write_error(error, error_capacity, "could not load pose labels");
        delete engine;
        return nullptr;
    }
    if (init_yolov8_pose_model(model_path, &engine->context) != 0) {
        write_error(error, error_capacity, "rknn model initialization failed");
        deinit_post_process();
        delete engine;
        return nullptr;
    }
    if (rknn_set_core_mask(engine->context.rknn_ctx, static_cast<rknn_core_mask>(core_mask)) != RKNN_SUCC) {
        write_error(error, error_capacity, "rknn_set_core_mask failed");
        release_yolov8_pose_model(&engine->context);
        deinit_post_process();
        delete engine;
        return nullptr;
    }
    return engine;
}

extern "C" void rkserve_yolov8_pose_destroy(rkserve_yolov8_pose_engine* engine) {
    if (engine == nullptr) {
        return;
    }
    release_yolov8_pose_model(&engine->context);
    deinit_post_process();
    delete engine;
}

extern "C" int32_t rkserve_yolov8_pose_infer(
    rkserve_yolov8_pose_engine* engine,
    const uint8_t* rgb,
    int32_t width,
    int32_t height,
    rkserve_pose* poses,
    int32_t capacity,
    char* error,
    size_t error_capacity) {
    if (engine == nullptr || rgb == nullptr || width <= 0 || height <= 0 || poses == nullptr) {
        write_error(error, error_capacity, "invalid inference input");
        return -1;
    }
    image_buffer_t image{};
    image.width = width;
    image.height = height;
    image.format = IMAGE_FORMAT_RGB888;
    image.virt_addr = const_cast<uint8_t*>(rgb);
    image.size = width * height * 3;

    object_detect_result_list results{};
    if (inference_yolov8_pose_model(&engine->context, &image, &results) != 0) {
        write_error(error, error_capacity, "RKNN inference failed");
        return -1;
    }
    const int32_t count = results.count < capacity ? results.count : capacity;
    for (int32_t index = 0; index < count; ++index) {
        const auto& source = results.results[index];
        poses[index].confidence = source.prop;
        poses[index].left = source.box.left;
        poses[index].top = source.box.top;
        poses[index].right = source.box.right;
        poses[index].bottom = source.box.bottom;
        for (int32_t point = 0; point < 17; ++point) {
            poses[index].keypoints[point] = {
                source.keypoints[point][0],
                source.keypoints[point][1],
                source.keypoints[point][2],
            };
        }
    }
    return count;
}

extern "C" int32_t rkserve_yolov8_pose_versions(
    rkserve_yolov8_pose_engine* engine,
    char* api_version,
    size_t api_capacity,
    char* driver_version,
    size_t driver_capacity) {
    if (engine == nullptr) {
        return -1;
    }
    rknn_sdk_version version{};
    if (rknn_query(engine->context.rknn_ctx, RKNN_QUERY_SDK_VERSION, &version, sizeof(version)) != RKNN_SUCC) {
        return -1;
    }
    std::snprintf(api_version, api_capacity, "%s", version.api_version);
    std::snprintf(driver_version, driver_capacity, "%s", version.drv_version);
    return 0;
}
