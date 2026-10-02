#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
# 基于 Ultralytics 8.4.103 Exporter 的 RKNN 流程（Ultralytics，AGPL-3.0）。
# 改动：显式简化失败即终止、统一元数据、检查 SDK 返回码、固定发布目录。
from datetime import datetime, timezone
from pathlib import Path
import os
import shutil
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
from conversion_common import build_rknn, fetch_inputs, inspect_rknn, onnx_shapes, retain_git_source


def main():
    recipe = Path(__file__).resolve().parent
    out = Path(sys.argv[1]).resolve()
    files = fetch_inputs(recipe, out)
    # wheel 中的代码不能替代完整上游源码包；另外保留 v8.4.103 的完整 git archive。
    retain_git_source(out, "https://github.com/ultralytics/ultralytics.git",
                      "bdc65f629a533f494da2a73cb8e201ea11943b54",
                      "dd8fd8b61cae9d046aad3f34f1d0ac48ec8d2b249ee20150c0764f401ec58abf", "ultralytics")
    import onnx
    import onnxslim
    import ultralytics
    import yaml
    from ultralytics import YOLO
    from ultralytics.engine.exporter import Exporter
    if ultralytics.__version__ != "8.4.103":
        raise RuntimeError("Ultralytics version mismatch")

    class CheckedExporter(Exporter):
        def export_rknn(self):
            stem = self.file.stem
            directory = out / "assets/models" / f"{stem}_rknn_fp16_640"
            directory.mkdir(parents=True)
            self.metadata["description"] = f"Ultralytics YOLO26{stem[-1]} model trained on the COCO dataset (coco.yaml)"
            # 默认 epoch=0，避免导出时间和上游作者路径污染可复现产物。
            self.metadata["date"] = datetime.fromtimestamp(int(os.environ["SOURCE_DATE_EPOCH"]), timezone.utc).replace(tzinfo=None).isoformat()
            if self.metadata["end2end"] or self.metadata["task"] != "detect":
                raise RuntimeError("A non-end2end detect model is required")
            self.args.opset = 19
            self.im = self.im[:1]
            # 上游 export_onnx 会捕获并忽略 simplifier 异常，故在此显式执行且不捕获。
            self.args.simplify = False
            source = Path(self.export_onnx())
            self.args.simplify = True
            graph = onnxslim.slim(onnx.load(str(source)))
            onnx.checker.check_model(graph)
            onnx.save(graph, str(source))
            if onnx_shapes(source) != ([[1, 3, 640, 640]], [[1, 84, 8400]]):
                raise RuntimeError(f"{stem} ONNX shape mismatch")
            target = directory / f"{stem}-rk3576.rknn"
            build_rknn(source, target,
                       config={"mean_values": [[0, 0, 0]], "std_values": [[255, 255, 255]],
                               "optimization_level": 3, "float_dtype": "float16"},
                       build={"rknn_batch_size": 1})
            inspect_rknn(target, [[1, 3, 640, 640]], [[1, 84, 8400]])
            (directory / "metadata.yaml").write_text(yaml.safe_dump(self.metadata, sort_keys=False, allow_unicode=True))
            return str(directory)

    for size in "nsmlx":
        name = f"yolo26{size}.pt"
        local = Path(os.environ["CONVERSION_WORK_DIR"]) / name
        shutil.copy2(files[name], local)
        model = YOLO(str(local), task="detect")
        exporter = CheckedExporter(overrides={"format": "rknn", "name": "rk3576", "device": "cpu",
                                              "imgsz": 640, "batch": 1, "quantize": 16,
                                              "opset": 19, "dynamic": False, "nms": False,
                                              "end2end": False, "simplify": True})
        exporter(model=model.model)
        del exporter, model


if __name__ == "__main__":
    main()
