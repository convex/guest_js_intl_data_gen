import tempfile
import unittest
from pathlib import Path

import check_generated as check
import tomllib


class ConsumerCheckTests(unittest.TestCase):
    def test_exporter_required_crates(self) -> None:
        for source in (
            '/// * `tinystr`\n/// * `icu_provider/baked`\n/// * `alloc`',
            '#[doc = " * `tinystr`"] #[doc = " * `icu_provider/baked`"]',
        ):
            self.assertEqual(check.required_crates(source), {"tinystr": set(), "icu_provider": {"baked"}})

    def test_dependencies_use_resolved_versions_and_features(self) -> None:
        metadata: dict = {
            "packages": [
                {"id": "generator", "name": "generator", "edition": "2024"},
                {"id": "tinystr", "name": "tinystr", "version": "0.8.4", "source": check.CRATES_IO},
            ],
            "resolve": {
                "root": "generator",
                "nodes": [
                    {"id": "generator", "features": []},
                    {"id": "tinystr", "features": ["zerovec"]},
                ],
            },
        }
        manifest = tomllib.loads(check.consumer_manifest(metadata, {"tinystr": {"alloc"}}))
        self.assertEqual(manifest["dependencies"]["tinystr"], {
            "package": "tinystr", "version": "=0.8.4", "default-features": False,
            "features": ["alloc", "zerovec"],
        })

    def test_consumer_lockfile_preserves_dependency_versions(self) -> None:
        dependency = f'''[[package]]
name = "tinystr"
version = "0.8.4"
source = "{check.CRATES_IO}"
checksum = "fixture-checksum"
'''
        with tempfile.TemporaryDirectory() as directory:
            original = Path(directory) / "original.lock"
            consumer = Path(directory) / "consumer.lock"
            original.write_text('[[package]]\nname = "generator"\nversion = "0.1.0"\n' + dependency)
            consumer.write_text('[[package]]\nname = "generated_data_check"\nversion = "0.0.0"\n' + dependency)
            check.check_lockfile(original, consumer)
            consumer.write_text(consumer.read_text().replace("0.8.4", "0.8.5"))
            with self.assertRaisesRegex(ValueError, "changed dependency"):
                check.check_lockfile(original, consumer)


if __name__ == "__main__":
    unittest.main()
