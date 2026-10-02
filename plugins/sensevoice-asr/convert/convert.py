#!/usr/bin/env python3
# 基于固定 lovemefan ONNX 的转换；不把第三方导出声明为 Alibaba 原始源码。
from pathlib import Path
import os
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
from conversion_common import build_rknn, fetch_inputs, inspect_rknn, verify


def main():
    recipe = Path(__file__).resolve().parent
    out = Path(sys.argv[1]).resolve()
    files = fetch_inputs(recipe, out)
    models, text = out / "assets/models", out / "assets/text"
    models.mkdir(parents=True)
    text.mkdir(parents=True)
    for name in ("am.mvn", "embedding.npy"):
        shutil.copy2(files[name], text / name)
    import sentencepiece as spm
    processor = spm.SentencePieceProcessor(model_file=str(files["chn_jpn_yue_eng_ko_spectok.bpe.model"]))
    if processor.get_piece_size() != 25055:
        raise RuntimeError("Unexpected SentencePiece vocabulary size")
    (text / "tokens.txt").write_text(
        "".join(f"{processor.id_to_piece(i)} {i}\n" for i in range(processor.get_piece_size())), encoding="utf-8")
    verify(text / "tokens.txt", "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc")
    fixed = Path(os.environ["CONVERSION_WORK_DIR"]) / "sense-voice-encoder.fixed.onnx"
    subprocess.run([sys.executable, str(recipe / "vendor/sv_fix_shape.py"),
                    str(files["sense-voice-encoder.onnx"]), str(fixed), "344"], check=True)
    verify(fixed, "e81639fff8e672475bf550f1f681c0da10361cd7c5f3ddee1d34956788fb428d")
    target = models / "sense-voice-encoder.rk3576.fp16.rknn"
    build_rknn(fixed, target, config={"optimization_level": 3, "float_dtype": "float16"},
               load={"inputs": ["speech"], "input_size_list": [[1, 344, 560]]})
    inspect_rknn(target, [[1, 344, 560]], [[1, 344, 25055]])


if __name__ == "__main__":
    main()
