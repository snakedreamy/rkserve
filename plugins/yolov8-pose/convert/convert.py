#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# RKNN 混合量化调用及区间依据 aiRockchip/rknn_model_zoo（Apache-2.0）。
# 提交 bad6c7334531becaf90a561988519b7bec34d0ab，examples/yolov8_pose/python/convert.py。
# 改动：固定输入、显式输出目录、验证所有返回码与混合量化输出契约。
from pathlib import Path
import os
import shutil
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
from conversion_common import checked, fetch_inputs, inspect_rknn, onnx_shapes


def main():
    recipe = Path(__file__).resolve().parent
    out = Path(sys.argv[1]).resolve()
    files = fetch_inputs(recipe, out)
    source = files["yolov8n-pose.onnx"]
    inputs, outputs = onnx_shapes(source)
    expected = [[1, 65, 80, 80], [1, 65, 40, 40], [1, 65, 20, 20], [1, 17, 3, 8400]]
    if inputs != [[1, 3, 640, 640]] or outputs != expected:
        raise RuntimeError("Expected the Rockchip four-output Pose ONNX")
    models = out / "assets/models"
    models.mkdir(parents=True)
    shutil.copy2(files["yolov8_pose_labels_list.txt"], models / "yolov8_pose_labels_list.txt")
    if (models / "yolov8_pose_labels_list.txt").read_bytes() != b"person\n":
        raise RuntimeError("Unexpected Pose label file")
    # 保留原校准清单；另生成 SDK 使用的绝对路径版，严格维持原顺序。
    paths = []
    for line in files["coco_subset_20.txt"].read_text().splitlines():
        key = line.removeprefix("./")
        if key not in files:
            raise RuntimeError(f"Missing calibration image: {key}")
        paths.append(str(files[key]))
    if len(paths) != 20 or len(set(paths)) != 20:
        raise RuntimeError("Exactly 20 distinct calibration images are required")
    dataset = out / "licenses/model-sources/calibration-absolute.txt"
    dataset.write_text("\n".join(paths) + "\n")
    from rknn.api import RKNN
    rknn = RKNN(verbose=True)
    target = models / "yolov8_pose.rknn"
    try:
        checked(rknn.config(target_platform="rk3576", mean_values=[[0, 0, 0]],
                            std_values=[[255, 255, 255]], optimization_level=3,
                            float_dtype="float16"), "config")
        checked(rknn.load_onnx(model=str(source)), "load_onnx")
        checked(rknn.hybrid_quantization_step1(
            dataset=str(dataset), proposal=False,
            custom_hybrid=[[f"/model.22/cv4.{i}/cv4.{i}.0/act/Mul_output_0",
                            "/model.22/Concat_6_output_0"] for i in range(3)]), "hybrid_quantization_step1")
        # SDK 在当前工作目录按输入 basename 生成这三个文件。
        intermediate = [Path(os.environ["CONVERSION_WORK_DIR"]) / f"yolov8n-pose{suffix}" for suffix in (".model", ".data", ".quantization.cfg")]
        for path in intermediate:
            if not path.is_file() or not path.stat().st_size:
                raise RuntimeError(f"Missing hybrid-quantization intermediate: {path}")
        checked(rknn.hybrid_quantization_step2(model_input=str(intermediate[0]),
                    data_input=str(intermediate[1]), model_quantization_cfg=str(intermediate[2])),
                "hybrid_quantization_step2")
        checked(rknn.export_rknn(str(target)), "export_rknn")
    finally:
        checked(rknn.release(), "release", allow_none=True)
    metadata = inspect_rknn(target, inputs, outputs)
    tensors = {tensor["tensor_id"]: tensor for tensor in metadata["norm_tensor"]}
    edges = sorted((edge for edge in metadata["graph"] if edge["left"] == "output"), key=lambda edge: edge["left_tensor_id"])
    quant = [tensors[edge["right_tensor_id"]]["dtype"]["qnt_type"] for edge in edges]
    if quant[:3] != ["int8"] * 3 or quant[3]:
        raise RuntimeError(f"Expected three INT8 detection outputs and unquantized keypoints, got {quant}")


if __name__ == "__main__":
    main()
