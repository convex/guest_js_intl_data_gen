import hashlib
import json
import os
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from typing import Any

from package import ASSET, NOTICES, ROOT, package


def _extract(tar: tarfile.TarFile, name: str) -> Any:
    member = tar.extractfile(name)
    assert member is not None
    return member


class PackagingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.data = self.root / "data"
        (self.data / "icu/nested").mkdir(parents=True)
        (self.data / "locales.rs").write_bytes(b'pub const AVAILABLE_LOCALES: &[&str] = &["en-US"];\n')
        (self.data / "icu/mod.rs").write_bytes(b"// fixture\n")
        (self.data / "icu/nested/data.rs").write_bytes(b"// nested fixture\n")
        self.generation: dict[str, Any] = {
            "format_version": 1,
            "source_tags": {
                "cldr": "fixture-cldr",
                "icuexport": "fixture-icuexport",
                "segmenter_lstm": "fixture-segmenter",
                "unicode": "fixture-unicode",
                "tzdb": "fixture-tzdb",
            },
            "icu4x_locales_override": "fr,en",
            "requested_locales": ["en", "en-US", "fr"],
        }
        (self.data / "generation.json").write_text(json.dumps(self.generation), encoding="utf-8")
        self.commit = "0123456789abcdef0123456789abcdef01234567"

    def package(self, name: str = "dist") -> Path:
        return package(self.data, self.root / name, self.commit)

    def test_archive_contract_and_provenance(self) -> None:
        archive = self.package()
        with tarfile.open(archive, "r:gz") as tar:
            names = tar.getnames()
            self.assertEqual(names, sorted([*NOTICES, "manifest.json", "locales.rs", "icu/mod.rs", "icu/nested/data.rs"]))
            for member in tar.getmembers():
                self.assertTrue(member.isfile())
                self.assertEqual((member.uid, member.gid, member.mtime, member.mode), (0, 0, 0, 0o644))
                self.assertEqual((member.uname, member.gname), ("", ""))
            manifest = json.load(_extract(tar, "manifest.json"))
            self.assertEqual(manifest["generator_commit"], self.commit)
            self.assertEqual(manifest["generation"], self.generation)

            self.assertEqual(set(manifest["files_sha256"]), set(names) - {"manifest.json"})
            for name, digest in manifest["files_sha256"].items():
                self.assertEqual(hashlib.sha256(_extract(tar, name).read()).hexdigest(), digest)
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        self.assertEqual((archive.parent / "SHA256SUMS").read_text(), f"{digest}  {ASSET}\n")
        notes = (archive.parent / "RELEASE_NOTES.md").read_text()
        self.assertIn(self.commit, notes)
        for field in ("source_tags", "icu4x_locales_override", "requested_locales"):
            self.assertIn(json.dumps(self.generation[field], sort_keys=True), notes)

        header = archive.read_bytes()[:10]
        self.assertEqual(header[3], 0, "gzip must not store a filename")
        self.assertEqual(header[4:8], bytes(4), "gzip timestamp must be zero")

    def test_deterministic_despite_paths_timestamps_and_permissions(self) -> None:
        first = self.package("first").read_bytes()
        for path in self.data.rglob("*"):
            os.utime(path, (123456789, 123456789))
            if path.is_file():
                path.chmod(0o600)
        other = self.root / "renamed-input"
        self.data.rename(other)
        self.data = other
        self.assertEqual(first, self.package("second").read_bytes())

    def test_default_locale_metadata(self) -> None:
        self.generation["icu4x_locales_override"] = None
        (self.data / "generation.json").write_text(json.dumps(self.generation), encoding="utf-8")
        with tarfile.open(self.package(), "r:gz") as tar:
            manifest = json.load(_extract(tar, "manifest.json"))
        self.assertEqual(manifest["generation"], self.generation)

    def test_cli_uses_generation_metadata_not_environment(self) -> None:
        destination = self.root / "cli"
        result = subprocess.run(
            [sys.executable, str(ROOT / "scripts/package.py"), str(self.data), str(destination),
             "--commit", self.commit],
            env={**os.environ, "ICU4X_LOCALES": "de,ja"},
            check=True, capture_output=True, text=True, timeout=30,
        )
        self.assertEqual(result.stdout.strip(), str(destination / ASSET))
        with tarfile.open(destination / ASSET, "r:gz") as tar:
            manifest = json.load(_extract(tar, "manifest.json"))
        self.assertEqual(manifest["generation"], self.generation)

    def test_incomplete_output_is_rejected(self) -> None:
        for name in ("locales.rs", "icu/mod.rs", "generation.json"):
            with self.subTest(name=name):
                path = self.data / name
                content = path.read_bytes()
                path.unlink()
                with self.assertRaises(ValueError):
                    self.package()
                path.write_bytes(content)

    def test_truncated_generation_metadata_is_rejected(self) -> None:
        (self.data / "generation.json").write_bytes(b"{")
        with self.assertRaisesRegex(ValueError, "generation.json"):
            self.package()

    def test_existing_archive_is_preserved(self) -> None:
        archive = self.package()
        before = archive.read_bytes()
        with self.assertRaises(FileExistsError):
            self.package()
        self.assertEqual(before, archive.read_bytes())


    def test_symlinks_are_rejected(self) -> None:
        (self.data / "icu/link").symlink_to(self.data / "locales.rs")
        with self.assertRaisesRegex(ValueError, "regular file"):
            self.package()


    def test_incomplete_commit_is_rejected(self) -> None:
        self.commit = "0123456"
        with self.assertRaisesRegex(ValueError, "commit"):
            self.package()


if __name__ == "__main__":
    unittest.main()
