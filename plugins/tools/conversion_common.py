"""转换专用的小工具：校验下载、SDK 返回码、静态契约和源码保留。"""
from __future__ import annotations

import hashlib
import json
import os
import shutil
import struct
import subprocess
import sys
import time
import urllib.request
from pathlib import Path


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def verify(path: Path, digest: str) -> Path:
    if not path.is_file() or path.stat().st_size == 0 or sha256(path) != digest:
        raise RuntimeError(f"Missing, empty, or SHA256-mismatched input: {path}")
    return path


def download(url: str, dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    curl = shutil.which("curl")
    if curl:
        subprocess.run([
            curl, "--fail", "--location", "--retry", "8", "--retry-delay", "5",
            "--connect-timeout", "30", "--max-time", "900",
            "--user-agent", "RKServe-model-conversion", "--output", str(dest), url,
        ], check=True)
        return
    last_error: Exception | None = None
    for attempt in range(1, 6):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": "RKServe-model-conversion"})
            with urllib.request.urlopen(request, timeout=120) as response, dest.open("wb") as stream:
                shutil.copyfileobj(response, stream, length=1024 * 1024)
            return
        except Exception as error:
            last_error = error
            dest.unlink(missing_ok=True)
            if attempt == 5:
                break
            time.sleep(min(60, 5 * attempt))
    raise RuntimeError(f"Download failed: {url}") from last_error


def fetch_inputs(recipe: Path, out: Path) -> dict[str, Path]:
    items = json.loads((recipe / "inputs.json").read_text())
    paths = {}
    for item in items:
        relative = Path(item["path"])
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError("Input filename must stay within licenses/model-sources/inputs")
        path = out / "licenses/model-sources/inputs" / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        if path.exists():
            try:
                verify(path, item["sha256"])
            except RuntimeError:
                path.unlink()
        if not path.exists():
            last_error: Exception | None = None
            for url in [item["url"], *item.get("urls", [])]:
                print(f"Download and verify {url}", flush=True)
                temporary = path.with_name(path.name + ".partial")
                temporary.unlink(missing_ok=True)
                try:
                    download(url, temporary)
                    verify(temporary, item["sha256"])
                    temporary.rename(path)
                    last_error = None
                    break
                except Exception as error:
                    last_error = error
                    temporary.unlink(missing_ok=True)
                    print(f"Download failed: {error}", flush=True)
            if last_error is not None:
                raise last_error
        verify(path, item["sha256"])
        paths[item["path"]] = path
    shutil.copy2(recipe / "inputs.json", out / "licenses/model-sources/INPUTS.json")
    return paths


def retain_git_source(out: Path, url: str, commit: str, digest: str, name: str) -> None:
    """保留完整上游源码；检查固定提交与 git archive 的预先核验 SHA。"""
    repo = Path(os.environ["CONVERSION_WORK_DIR"]) / f"{name}-git"
    subprocess.run(["git", "init", str(repo)], check=True)
    subprocess.run(["git", "-C", str(repo), "fetch", "--depth=1", url, commit], check=True)
    actual = subprocess.check_output(["git", "-C", str(repo), "rev-parse", "FETCH_HEAD"], text=True).strip()
    if actual != commit:
        raise RuntimeError("Upstream source commit mismatch")
    target = out / "licenses/model-sources" / f"{name}-{commit}.tar"
    with target.open("wb") as stream:
        subprocess.run(["git", "-C", str(repo), "archive", "--format=tar", commit], stdout=stream, check=True)
    verify(target, digest)
    (out / "licenses/model-sources" / f"{name}-SOURCE.json").write_text(
        json.dumps(dict(url=url, commit=commit, sha256=digest), indent=2) + "\n")


def checked(ret, operation: str, *, allow_none: bool = False) -> None:
    if ret != 0 and not (allow_none and ret is None):
        raise RuntimeError(f"RKNN {operation} failed with return value {ret!r}")


def build_rknn(onnx: Path, output: Path, *, config: dict, load: dict | None = None,
               build: dict | None = None) -> None:
    from rknn.api import RKNN
    output.parent.mkdir(parents=True, exist_ok=True)
    rknn = RKNN(verbose=True)
    try:
        checked(rknn.config(target_platform="rk3576", **config), "config")
        checked(rknn.load_onnx(model=str(onnx), **(load or {})), "load_onnx")
        checked(rknn.build(do_quantization=False, **(build or {})), "build")
        checked(rknn.export_rknn(str(output)), "export_rknn")
    finally:
        # release 在此 SDK 中没有规定整数返回值。
        checked(rknn.release(), "release", allow_none=True)
    if not output.is_file() or not output.stat().st_size:
        raise RuntimeError(f"SDK did not produce a model: {output}")


def inspect_rknn(path: Path, inputs: list[list[int]], outputs: list[list[int]]) -> dict:
    """只读文件头，不加载运行时、不初始化 NPU。"""
    with path.open("rb") as stream:
        header = stream.read(64)
        if len(header) != 64 or header[:4] != b"RKNN" or struct.unpack_from("<Q", header, 8)[0] != 6:
            raise RuntimeError(f"Unexpected RKNN header: {path}")
        offset = 64 + struct.unpack_from("<Q", header, 16)[0]
        stream.seek(offset)
        length_bytes = stream.read(8)
        if len(length_bytes) != 8:
            raise RuntimeError("Missing RKNN trailer")
        length = struct.unpack("<Q", length_bytes)[0]
        if length > 16 * 1024 * 1024 or offset + 8 + length != path.stat().st_size:
            raise RuntimeError("Invalid RKNN metadata length")
        metadata = json.loads(stream.read(length))
    if metadata["version"] != "2.3.2" or metadata["target_platform"] != ["rk3576"]:
        raise RuntimeError("RKNN version or target platform mismatch")
    tensors = {tensor["tensor_id"]: tensor for tensor in metadata["norm_tensor"]}
    for kind, expected in (("input", inputs), ("output", outputs)):
        edges = sorted((e for e in metadata["graph"] if e["left"] == kind), key=lambda e: e["left_tensor_id"])
        actual = [tensors[e["right_tensor_id"]]["size"] for e in edges]
        # RKNN 把 ONNX scalar 暴露为单元素向量。
        if actual != [shape or [1] for shape in expected]:
            raise RuntimeError(f"{path.name} {kind} shape mismatch: {actual}; expected {expected}")
    return metadata


def onnx_shapes(path: Path) -> tuple[list[list[int]], list[list[int]]]:
    import onnx
    model = onnx.load(str(path), load_external_data=False)
    def shapes(values):
        result = []
        for value in values:
            dims = value.type.tensor_type.shape.dim
            if any(not d.HasField("dim_value") or d.dim_value <= 0 for d in dims):
                raise RuntimeError(f"Static ONNX dimensions required: {value.name}")
            result.append([d.dim_value for d in dims])
        return result
    return shapes(model.graph.input), shapes(model.graph.output)


def write_hashes(root: Path, files: list[Path], target: Path) -> None:
    target.write_text("".join(f"{sha256(p)}  {p.relative_to(root).as_posix()}\n" for p in sorted(files)))


def finalize(recipe: Path, out: Path) -> None:
    # 产物存在还不够；原始输入、源码和环境证据缺失时同样不能发布。
    for item in json.loads((recipe / "inputs.json").read_text()):
        verify(out / "licenses/model-sources/inputs" / item["path"], item["sha256"])
    required = ["licenses/model-sources/INPUTS.json", "licenses/model-sources/environment/requirements.txt",
                "licenses/model-sources/environment/pip-freeze.txt", "licenses/model-sources/environment/python.txt",
                "licenses/model-sources/environment/os-release", "licenses/model-sources/conversion/plugins/tools/conversion_common.py",
                f"licenses/model-sources/conversion/plugins/{recipe.parent.name}/convert/convert.py"]
    for relative in required:
        path = out / relative
        if not path.is_file() or not path.stat().st_size:
            raise RuntimeError(f"Missing reproducible-build evidence: {path}")
    if not list((out / "licenses/model-sources/environment/wheels").glob("*.whl")):
        raise RuntimeError("Environment wheels were not retained")
    expected = json.loads((recipe / "outputs.json").read_text())
    actual = {p.relative_to(out).as_posix() for p in (out / "assets").rglob("*") if p.is_file()}
    if actual != set(expected):
        raise RuntimeError(f"Artifact set mismatch; missing {set(expected) - actual}; unexpected {actual - set(expected)}")
    for relative in expected:
        path = out / relative
        if path.is_symlink() or path.stat().st_size == 0:
            raise RuntimeError(f"Artifact is not a nonempty regular file: {path}")
    # 原始下载、转换源码、许可证、wheel 和中间图均保留；虚拟环境不属于源码包。
    intermediates = out / "licenses/model-sources/intermediates"
    intermediates.mkdir()
    for suffix in ("*.onnx", "*.model", "*.data", "*.quantization.cfg"):
        for path in (Path(os.environ["CONVERSION_WORK_DIR"])).glob(suffix):
            shutil.copy2(path, intermediates / path.name)
    source_files = [p for p in (out / "licenses/model-sources").rglob("*") if p.is_file()]
    write_hashes(out, source_files, out / "licenses/model-sources/SHA256SUMS")
    record = (
        f"Plugin: {recipe.parent.name}\nTarget: rk3576 / rknn-toolkit2 2.3.2 / Python 3.11 x86_64\n"
        "Input URLs, versions, licenses, and SHA256: licenses/model-sources/INPUTS.json\n"
        "Executed conversion source and documented gaps: licenses/model-sources/conversion/\n"
        "Complete dependency wheels, pins, and environment: licenses/model-sources/environment/\n"
        "Original model/data inputs and retained source: licenses/model-sources/inputs/ and licenses/model-sources/\n"
        "Retained input/source SHA256: licenses/model-sources/SHA256SUMS\n"
        "Output SHA256: licenses/OUTPUTS.sha256\n"
        "Notice: conversion from published ONNX/PT is not proof of complete training source or GPL/AGPL compliance.\n\n"
    )
    (out / "licenses/MODEL-SOURCES.txt").write_text(record + (recipe / "README.md").read_text())
    files = [p for base in ("assets", "licenses") for p in (out / base).rglob("*")
             if p.is_file() and p.name != "CONVERSION-INCOMPLETE"]
    write_hashes(out, files, out / "licenses/OUTPUTS.sha256")


if __name__ == "__main__":
    if len(sys.argv) != 4 or sys.argv[1] != "finalize":
        raise SystemExit("Usage: conversion_common.py finalize <recipe> <output>")
    finalize(Path(sys.argv[2]), Path(sys.argv[3]))
