#!/usr/bin/env python3
# 沿用 Rockchip 发布的静态 ONNX；不冒充 k2 原始 ONNX 的静态化实现。
from pathlib import Path
import os
import shutil
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
from conversion_common import build_rknn, fetch_inputs, inspect_rknn, onnx_shapes


def main():
    recipe = Path(__file__).resolve().parent
    out = Path(sys.argv[1]).resolve()
    files = fetch_inputs(recipe, out)
    models, text = out / "assets/models", out / "assets/text"
    models.mkdir(parents=True)
    text.mkdir(parents=True)
    shutil.copy2(files["vocab.txt"], text / "vocab.txt")
    for name in ("encoder", "decoder", "joiner"):
        source = files[f"{name}-epoch-99-avg-1.onnx"]
        inputs, outputs = onnx_shapes(source)
        if name == "encoder" and (len(inputs) != 36 or inputs[0] != [1, 103, 80] or len(outputs) != 36):
            raise RuntimeError("Encoder state/frame contract mismatch")
        if name == "decoder" and (inputs != [[1, 2]] or outputs != [[1, 512]]):
            raise RuntimeError("Decoder contract mismatch")
        if name == "joiner" and (inputs != [[1, 512], [1, 512]] or outputs != [[1, 6254]]):
            raise RuntimeError("Joiner contract mismatch")
        # 固定 toolkit 的默认 opt3/fp16；与上游仅设置 target 的调用等价。
        target = models / f"{name}.rknn"
        build_rknn(source, target, config={"optimization_level": 3, "float_dtype": "float16"})
        inspect_rknn(target, inputs, outputs)


if __name__ == "__main__":
    main()
