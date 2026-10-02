#!/usr/bin/env python3
# Matcha/Vocos 的已发布 ONNX 转换；不是原始训练工程。
from pathlib import Path
import os
import shutil
import subprocess
import sys
import tarfile

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
from conversion_common import build_rknn, fetch_inputs, inspect_rknn, verify


def main():
    recipe = Path(__file__).resolve().parent
    out = Path(sys.argv[1]).resolve()
    files = fetch_inputs(recipe, out)
    work = Path(os.environ["CONVERSION_WORK_DIR"])
    models = out / "assets/models"
    text = out / "assets/text"
    models.mkdir(parents=True)
    text.mkdir(parents=True)
    # 只提取所需普通文件，拒绝归档链接与路径逃逸。
    # eSpeak 数据由 ARM64 Worker 构建生成，不用旧归档覆盖新编译的数据。
    names = ["model-steps-3.onnx", "lexicon.txt", "tokens.txt"]
    with tarfile.open(files["matcha-icefall-zh-en.tar.bz2"], "r:bz2") as archive:
        for name in names:
            member = archive.getmember("matcha-icefall-zh-en/" + name)
            if not member.isfile():
                raise RuntimeError(f"Archive member is not a regular file: {name}")
            target = work / name if name.endswith(".onnx") else text / name
            target.parent.mkdir(parents=True, exist_ok=True)
            with archive.extractfile(member) as source, target.open("wb") as destination:
                shutil.copyfileobj(source, destination)
    original = verify(work / "model-steps-3.onnx", "524286bf6cf11be74329ae1c682ac69e34d6860c2ea9fd1290319d561540b16a")
    verify(text / "lexicon.txt", "599efdcdaff4df2a123ce988c2cb90abcd59b919ef609ce36eb587d68b7ca2c0")
    verify(text / "tokens.txt", "77fee8e5e5dd96b3547119e6159292c648e99e065c54c97777722e3ce710b9a4")
    import onnx
    from onnx import TensorProto, helper
    model = onnx.load(str(original))
    model.graph.output.append(helper.make_tensor_value_info("/ReduceMax_output_0", TensorProto.INT64, []))
    with_length = work / "matcha-with-length.onnx"
    onnx.save(model, str(with_length))
    verify(with_length, "6d07c540de27a2d17bbbe7e8feaf3c7ac690110de0c1e9e8cc35e94117a066d0")
    # 必须从加过长度输出的图提取，否则不能复现原 duration 文件。
    duration = models / "matcha-duration.onnx"
    onnx.utils.extract_model(str(with_length), str(duration),
                             ["x", "x_length", "length_scale"], ["/ReduceMax_output_0"])
    verify(duration, "b5571a905d8fe75de28e685cc4c952dfca727b49c247a1d94f4bc3d911c537b6")
    fixed = work / "matcha-s64-length-fixed.onnx"
    subprocess.run([sys.executable, str(recipe / "vendor/fix_matcha_rknn.py"),
                    "--input", str(with_length), "--output", str(fixed),
                    "--seq-len", "80", "--x-len", "64"], check=True)
    # Probe-first rewrite is checked by ORT inside the vendor script; its bytes are not a pinned input.
    if not fixed.is_file() or fixed.stat().st_size == 0:
        raise RuntimeError(f"Graph rewrite produced no model: {fixed}")
    matcha = models / "matcha-s64-fp16.rknn"
    build_rknn(fixed, matcha, config={"optimization_level": 0, "float_dtype": "float16"})
    inspect_rknn(matcha, [[1, 80], [1], [1], [1]], [[1, 80, 599], []])
    vocos = models / "vocos-16khz-600-fp16.rknn"
    build_rknn(files["vocos-16khz-univ.onnx"], vocos,
               config={"optimization_level": 3, "single_core_mode": False, "float_dtype": "float16"},
               load={"inputs": ["mels"], "input_size_list": [[1, 80, 600]]})
    inspect_rknn(vocos, [[1, 80, 600]], [[1, 513, 600]] * 3)
    # 完整 eSpeak 源归档同时置于许可目录，方便后续源码包保持关联。
    shutil.copy2(files["espeak-ng-source-ed530aa.zip"], out / "licenses/espeak-ng-source-ed530aa.zip")


if __name__ == "__main__":
    main()
