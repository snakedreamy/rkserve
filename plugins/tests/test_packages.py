"""Plugin install and packaging tests without models or worker compilation."""
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
INSTALL = ROOT / "plugins/install.sh"
MANIFEST = """\
schema_version = 2

[plugin]
id = "yolo26"
name = "YOLO26 Object Detection"
version = "0.1.0"
protocol_version = 2
executable = "bin/worker"

[[capabilities]]
id = "vision.detect"
name = "Object detection"
description = "test fixture"
input_kind = "image"
accepted_content_types = ["image/jpeg"]
max_input_bytes = 1048576
output_kind = "detections"
output_content_type = "application/json"

[resources.npu]
allowed_masks = ["core0"]
default_mask = "core0"
max_concurrency = 1
queue_size = 1
request_timeout_ms = 1000
"""


class PluginPackages(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "release"
        self.source.mkdir()
        self.destination = self.root / "installed"

    def archive(self, extra=None, worker_mode=0o755, manifest=MANIFEST):
        path = self.source / "rkserve-plugin-yolo26.tar.gz"
        with tarfile.open(path, "w:gz") as bundle:
            for name, content, mode in [
                ("yolo26/plugin.toml", manifest.encode(), 0o644),
                ("yolo26/bin/worker", b"#!/bin/sh\nexit 0\n", worker_mode),
            ]:
                member = tarfile.TarInfo(name)
                member.size = len(content)
                member.mode = mode
                bundle.addfile(member, io.BytesIO(content))
            if extra:
                bundle.addfile(extra)
        self.checksum(path)

    def checksum(self, path):
        (self.source / "SHA256SUMS").write_text(
            f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n"
        )

    def install(self, *ids):
        return subprocess.run(
            [str(INSTALL), "--dest", str(self.destination), "--from", str(self.source), *(ids or ["yolo26"])],
            capture_output=True, text=True,
        )

    def test_install_and_update(self):
        self.archive()
        self.assertEqual(self.install().returncode, 0)
        self.assertEqual(self.install().returncode, 0)
        self.assertTrue((self.destination / "yolo26/bin/worker").stat().st_mode & 0o111)

    def test_invalid_checksum_preserves_old_install(self):
        self.archive()
        self.assertEqual(self.install().returncode, 0)
        (self.source / "SHA256SUMS").write_text("0" * 64 + "  rkserve-plugin-yolo26.tar.gz\n")
        self.assertNotEqual(self.install().returncode, 0)
        self.assertTrue((self.destination / "yolo26/bin/worker").is_file())

    def test_invalid_manifest_preserves_old_install(self):
        self.archive()
        self.assertEqual(self.install().returncode, 0)
        old = (self.destination / "yolo26/bin/worker").read_bytes()
        cases = [
            '[plugin]\nid="yolo26"\n',
            MANIFEST.replace('id = "yolo26"', 'id = "sensevoice-asr"'),
            MANIFEST.replace("schema_version = 2", "schema_version = 1"),
            MANIFEST.replace("protocol_version = 2", "protocol_version = 1"),
        ]
        for manifest in cases:
            with self.subTest(manifest=manifest.splitlines()[0]):
                self.archive(manifest=manifest)
                self.assertNotEqual(self.install().returncode, 0)
                self.assertEqual((self.destination / "yolo26/bin/worker").read_bytes(), old)

    def test_reject_links_traversal_and_nonexecutable(self):
        for name, kind in [("yolo26/link", tarfile.SYMTYPE), ("../escaped", tarfile.REGTYPE), ("other/file", tarfile.REGTYPE)]:
            with self.subTest(name=name):
                extra = tarfile.TarInfo(name)
                extra.type = kind
                extra.linkname = "../../outside"
                self.archive(extra)
                self.assertNotEqual(self.install().returncode, 0)
                self.assertFalse((self.destination / "yolo26").exists())
        self.archive(worker_mode=0o644)
        self.assertNotEqual(self.install().returncode, 0)

    def test_duplicate_plugin_rejected(self):
        self.archive()
        self.assertNotEqual(self.install("yolo26", "yolo26").returncode, 0)

    def package_fixture(self, plugin_line="Plugin: yolo26", extra_model=None):
        worker = self.root / "worker"
        models = self.root / "models"
        if worker.exists():
            subprocess.run(["rm", "-rf", str(worker), str(models)], check=True)
        (worker / "bin").mkdir(parents=True)
        (worker / "lib").mkdir(parents=True)
        (worker / "bin/worker").write_text("test fixture")
        (worker / "lib/librknnrt.so").write_text("stub rknnrt")
        (worker / "lib/librga.so").write_text("stub rga")
        hashed = []
        for relative in json.loads((ROOT / "plugins/yolo26/convert/outputs.json").read_text()):
            path = models / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("stub " + relative)
            hashed.append(path)
        (models / "licenses").mkdir(parents=True, exist_ok=True)
        record = models / "licenses/MODEL-SOURCES.txt"
        record.write_text(plugin_line + "\ntest fixture provenance\n")
        hashed.append(record)
        (models / "licenses/CONVERT-NOTICE.txt").write_text("keep this license text\n")
        bulky = models / "licenses/model-sources/inputs/torch.whl"
        bulky.parent.mkdir(parents=True, exist_ok=True)
        bulky.write_text("not for the release tarball\n")
        if extra_model:
            extra_path = models / extra_model
            extra_path.parent.mkdir(parents=True, exist_ok=True)
            extra_path.write_text("unexpected")
        (models / "licenses/OUTPUTS.sha256").write_text("".join(
            f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.relative_to(models).as_posix()}\n"
            for path in hashed
        ))
        return [str(ROOT / "plugins/tools/package.sh"), "yolo26", str(worker), str(models), str(self.source)]

    def test_package_restores_artifact_permissions(self):
        subprocess.run(self.package_fixture(), check=True, capture_output=True)
        archive = self.source / "rkserve-plugin-yolo26.tar.gz"
        sidecar = self.source / "rkserve-plugin-yolo26.tar.gz.sha256"
        self.assertTrue(sidecar.is_file())
        self.assertIn(archive.name, sidecar.read_text())
        with tarfile.open(archive, "r:gz") as bundle:
            names = bundle.getnames()
            self.assertIn("yolo26/licenses/RKServe-NOTICE", names)
            self.assertIn("yolo26/licenses/MODEL-SOURCES.txt", names)
            self.assertIn("yolo26/licenses/CONVERT-NOTICE.txt", names)
            self.assertIn("yolo26/plugin.toml", names)
            self.assertFalse(any("model-sources" in name for name in names))
        self.assertEqual(self.install().returncode, 0)

    def test_package_rejects_cross_plugin_and_override(self):
        result = subprocess.run(self.package_fixture(plugin_line="Plugin: sensevoice-asr"), capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        result = subprocess.run(self.package_fixture(extra_model="bin/worker"), capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)

    def test_failed_compression_preserves_old_archive(self):
        command = self.package_fixture()
        subprocess.run(command, check=True, capture_output=True)
        archive = self.source / "rkserve-plugin-yolo26.tar.gz"
        original = archive.read_bytes()
        stubs = self.root / "stubs"
        stubs.mkdir()
        gzip = stubs / "gzip"
        gzip.write_text("#!/bin/sh\nprintf partial\nexit 1\n")
        gzip.chmod(0o755)
        result = subprocess.run(command, capture_output=True, env={
            **os.environ, "PATH": f"{stubs}:{os.environ['PATH']}",
        })
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(archive.read_bytes(), original)
        self.assertEqual(list(self.source.glob(".rkserve-plugin-*")), [])

    def test_conversion_rejects_symlink_to_existing_output(self):
        output = self.root / "output"
        output.mkdir()
        sentinel = output / "existing-model"
        sentinel.write_bytes(b"preserve old output")
        link = self.root / "output-link"
        link.symlink_to(output, target_is_directory=True)
        python = self.root / "python-stub"
        python.write_text('#!/bin/sh\n[ "$1" = "-c" ]\n')
        python.chmod(0o755)
        result = subprocess.run([
            "bash", str(ROOT / "plugins/tools/conversion-run.sh"),
            str(ROOT / "plugins/yolo26/convert"), str(link),
        ], capture_output=True, text=True, env={
            **os.environ, "CONVERSION_PYTHON": str(python),
        })
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Output directory must be absent or empty", result.stderr)
        self.assertEqual(list(output.iterdir()), [sentinel])
        self.assertEqual(sentinel.read_bytes(), b"preserve old output")


if __name__ == "__main__":
    unittest.main()
