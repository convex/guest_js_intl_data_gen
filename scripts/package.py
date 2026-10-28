"""Package completed generation output with normalized tar/gzip metadata (Python 3.11+)."""

import argparse
import gzip
import hashlib
import io
import json
import re
import stat
import tarfile
from pathlib import Path
from typing import Any

import tomllib

ROOT = Path(__file__).resolve().parents[1]
ASSET = "guest_js_intl_data.tar.gz"

NOTICES = ("LICENSE", "LICENSE-UNICODE", "LICENSE-TZDB", "THIRD_PARTY_NOTICES.md")


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def output_files(data: Path) -> dict[str, Path]:
    if data.is_symlink() or not data.is_dir():
        raise ValueError("data must be a directory, not a symlink")
    icu = data / "icu"
    if icu.is_symlink() or not icu.is_dir():
        raise ValueError("missing regular icu/ directory")
    files: dict[str, Path] = {"locales.rs": data / "locales.rs"}
    for path in sorted(icu.rglob("*")):
        mode = path.lstat().st_mode
        if stat.S_ISDIR(mode):
            continue
        if not stat.S_ISREG(mode):
            raise ValueError(f"expected regular file: {path}")
        files[path.relative_to(data).as_posix()] = path
    for path in (data / "locales.rs", icu / "mod.rs"):
        if not path.exists() or not stat.S_ISREG(path.lstat().st_mode):
            raise ValueError(f"missing regular file: {path}")
    return files


def read_generation(data: Path) -> dict[str, Any]:
    path = data / "generation.json"
    if path.is_symlink() or not path.is_file():
        raise ValueError(
            "missing regular generation.json file (symlinks are not allowed)"
        )
    try:
        generation = json.loads(path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise ValueError(f"invalid generation.json: {error}") from error
    if not isinstance(generation, dict):
        raise TypeError("generation.json must contain an object")
    version = generation.get("format_version")
    if type(version) is not int:
        raise ValueError("generation.json format_version must be an integer")
    if version != 1:
        raise ValueError(f"unsupported generation.json format_version: {version}")
    tags = generation.get("source_tags")
    if (
        not isinstance(tags, dict)
        or set(tags) != {"cldr", "icuexport", "segmenter_lstm", "unicode", "tzdb"}
        or not all(isinstance(tag, str) for tag in tags.values())
    ):
        raise ValueError(
            "generation.json source_tags must contain cldr, icuexport, segmenter_lstm, unicode, tzdb strings"
        )
    if "icu4x_locales_override" not in generation or (
        generation["icu4x_locales_override"] is not None
        and not isinstance(generation["icu4x_locales_override"], str)
    ):
        raise ValueError(
            "generation.json icu4x_locales_override must be a string or null"
        )
    locales = generation.get("requested_locales")
    if not isinstance(locales, list) or not all(
        isinstance(locale, str) for locale in locales
    ):
        raise ValueError(
            "generation.json requested_locales must be an array of strings"
        )
    return generation


def package(data: Path, destination: Path, commit: str) -> Path:
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("commit must be a full, lowercase 40-character Git commit SHA")
    files = output_files(data)
    generation = read_generation(data)
    files.update({name: ROOT / name for name in NOTICES})
    toolchain = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"][
        "channel"
    ]
    packages = tomllib.loads((ROOT / "Cargo.lock").read_text())["package"]
    provider_version = next(
        p["version"] for p in packages if p["name"] == "icu_provider_source"
    )
    manifest: dict[str, Any] = {
        "format_version": 1,
        "generator_repository": "https://github.com/convex/guest_js_intl_data_gen",
        "generator_commit": commit,
        "toolchain": toolchain,
        "input_sha256": {
            name: sha256(ROOT / name)
            for name in (
                "Cargo.toml",
                "Cargo.lock",
                "rust-toolchain.toml",
                "src/main.rs",
                "scripts/package.py",
            )
        },
        "icu_provider_source": provider_version,
        "generation": generation,
        "archive_metadata": "sorted paths; mtime/uid/gid=0; empty owner names; mode=0644; gzip mtime=0, no filename",
        "files_sha256": {name: sha256(path) for name, path in sorted(files.items())},
    }
    manifest_bytes = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode()
    notes = f"""# Generated ICU4X data

Generator commit: `{commit}`
Toolchain: `{toolchain}`
Source provider: `icu_provider_source {provider_version}`
Locale override (`ICU4X_LOCALES` during generation): `{json.dumps(generation["icu4x_locales_override"])}`
Requested locales: `{json.dumps(generation["requested_locales"])}`
Source tags: `{json.dumps(generation["source_tags"], sort_keys=True)}`

See `src/main.rs` at the generator commit for export settings and marker selection.

`guest_js_intl_data.tar.gz` contains `locales.rs`, `icu/`, license notices, and
`manifest.json` with embedded `generation.json` metadata, generator input hashes,
and per-file SHA-256 checksums.
Verify the archive with `sha256sum -c SHA256SUMS` before extracting it.
See the repository README at this commit for regeneration and licensing details.
"""
    destination.mkdir(parents=True, exist_ok=True)
    archive = destination / ASSET
    # Exclusive creation makes accidental repackaging into a published artifact directory fail.
    with (
        archive.open("xb") as output,
        gzip.GzipFile(
            filename="", mode="wb", compresslevel=9, mtime=0, fileobj=output
        ) as compressed,
        tarfile.open(fileobj=compressed, mode="w|", format=tarfile.PAX_FORMAT) as tar,
    ):
        for name in sorted([*files, "manifest.json"]):
            info = tarfile.TarInfo(name)
            info.mode = 0o644
            if name == "manifest.json":
                info.size = len(manifest_bytes)
                tar.addfile(info, io.BytesIO(manifest_bytes))
            else:
                info.size = files[name].stat().st_size
                with files[name].open("rb") as content:
                    tar.addfile(info, content)
    (destination / "SHA256SUMS").write_text(
        f"{sha256(archive)}  {ASSET}\n", encoding="utf-8"
    )
    (destination / "RELEASE_NOTES.md").write_text(notes, encoding="utf-8")
    return archive


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "data",
        type=Path,
        help="completed generator output directory (including generation.json)",
    )
    parser.add_argument(
        "destination", type=Path, help="artifact directory (archive must not exist)"
    )
    parser.add_argument("--commit", required=True, help="full generator Git commit SHA")
    args = parser.parse_args()
    try:
        archive = package(args.data, args.destination, args.commit)
    except (OSError, ValueError) as error:
        parser.exit(1, f"packaging failed: {error}\n")
    print(archive)


if __name__ == "__main__":
    main()
