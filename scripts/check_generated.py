"""Compile all baked provider implementations and locales.rs (Python 3.11+).

Run after generation: python3 scripts/check_generated.py data
No source-data downloads or generation are performed.
The exporter documents required crates/features in icu/mod.rs; their exact versions
and enabled features come from the generator's resolved Cargo graph, not new pins.

Only a temporary standalone consumer is written. Its copied lockfile is deliberately
reconciled by cargo metadata --offline (without --locked), then checked for dependency
version/source/checksum changes before cargo check --locked. Cargo runs
from the repository so rustup uses rust-toolchain.toml even for the temp manifest.
"""

import argparse
import json
import re
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Any

import tomllib

ROOT = Path(__file__).resolve().parents[1]
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"


def required_crates(source: str) -> dict[str, set[str]]:
    # BakedExporter::close writes the same list as its required_crates report into
    # doc attributes (or /// comments when pretty-printed), including crate/feature.
    requirements = re.findall(r"\*\s+`([A-Za-z_][A-Za-z0-9_]*(?:/[A-Za-z0-9_-]+)?)`", source)
    if not requirements:
        raise ValueError("icu/mod.rs has no exporter required-crates documentation")
    crates: dict[str, set[str]] = {}
    for requirement in requirements:
        name, _, feature = requirement.partition("/")
        if name in {"alloc", "core", "std"}:
            continue
        features = crates.setdefault(name, set())
        if feature:
            features.add(feature)
    return crates


def consumer_manifest(metadata: dict[str, Any], requirements: dict[str, set[str]]) -> str:
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    root = packages[metadata["resolve"]["root"]]
    lines = [
        "[package]",
        'name = "generated_data_check"',
        'version = "0.0.0"',
        f'edition = {json.dumps(root["edition"])}',
        "publish = false",
        "[workspace]",
        "[dependencies]",
    ]
    for name, extra_features in sorted(requirements.items()):
        matches = [p for p in packages.values() if p["id"] in nodes and p["name"].replace("-", "_") == name]
        if len(matches) != 1:
            raise ValueError(f"required crate {name!r} must resolve to exactly one generator package (found {len(matches)})")
        package = matches[0]
        source = package["source"]
        fields = [
            f'package = {json.dumps(package["name"])}',
            f'version = {json.dumps("=" + package["version"])}',
            "default-features = false",
            f'features = {json.dumps(sorted(set(nodes[package["id"]]["features"]) | extra_features))}',
        ]
        if source is None:
            fields.append(f'path = {json.dumps(str(Path(package["manifest_path"]).parent), ensure_ascii=False)}')
        elif source != CRATES_IO:
            raise ValueError(f"required crate {name!r} has unsupported source {source!r}; refusing to substitute crates.io")
        lines.append(f'{json.dumps(name)} = {{ {", ".join(fields)} }}')
    return "\n".join(lines) + "\n"


def rust_path(path: Path) -> str:
    hashes = "#"
    while '"' + hashes in str(path):
        hashes += "#"
    return f'r{hashes}"{path}"{hashes}'


def consumer_source(data: Path) -> str:
    # #[path] keeps include! paths relative to the generated mod.rs and avoids
    # macro-expanded-macro-export absolute-path errors from include!(mod.rs).
    # macro_use imports the private aggregate macro; the glob imports the public
    # per-marker aliases that it invokes. Merely loading mod.rs would not typecheck
    # any of those implementations.
    return f"""extern crate alloc;

#[macro_use]
#[path = {rust_path(data / 'icu/mod.rs')}]
mod baked;
use baked::*;

pub struct BakedDataProvider;
impl_data_provider!(BakedDataProvider);

#[path = {rust_path(data / 'locales.rs')}]
pub mod locales;
"""


def check_lockfile(original: Path, consumer: Path) -> None:
    def identity(package: dict[str, Any]) -> tuple[Any, ...]:
        return tuple(package.get(key) for key in ("name", "version", "source", "checksum"))

    allowed = {identity(p) for p in tomllib.loads(original.read_text())["package"]}
    for package in tomllib.loads(consumer.read_text())["package"]:
        if (package["name"], package["version"], package.get("source")) == ("generated_data_check", "0.0.0", None):
            continue
        if identity(package) not in allowed:
            raise ValueError(f"consumer lockfile changed dependency: {package['name']} {package['version']}.")


def cargo_metadata(root: Path, manifest: Path, *, locked: bool) -> dict[str, Any]:
    command = ["cargo", "metadata", "--format-version=1", "--offline", "--manifest-path", str(manifest)]
    if locked:
        command.append("--locked")
    return json.loads(subprocess.run(command, cwd=root, check=True, stdout=subprocess.PIPE, text=True).stdout)


def check_generated(data: Path, root: Path = ROOT) -> None:
    data = data.resolve()
    for relative in ("icu/mod.rs", "locales.rs"):
        if not (data / relative).is_file():
            raise ValueError(f"missing generated file: {data / relative}")
    requirements = required_crates((data / "icu/mod.rs").read_text())
    metadata = cargo_metadata(root, root / "Cargo.toml", locked=True)
    manifest = consumer_manifest(metadata, requirements)
    with tempfile.TemporaryDirectory(prefix="icu-generated-check-") as directory:
        consumer = Path(directory)
        (consumer / "Cargo.toml").write_text(manifest, encoding="utf-8")
        (consumer / "src").mkdir()
        (consumer / "src/lib.rs").write_text(consumer_source(data), encoding="utf-8")
        shutil.copyfile(root / "Cargo.lock", consumer / "Cargo.lock")
        print("Reconciling temporary copied Cargo.lock with cargo metadata.", flush=True)
        cargo_metadata(root, consumer / "Cargo.toml", locked=False)
        check_lockfile(root / "Cargo.lock", consumer / "Cargo.lock")
        print("Checking all baked implementations and locales.rs with cargo check --locked.", flush=True)
        subprocess.run(
            ["cargo", "check", "--locked", "--manifest-path", str(consumer / "Cargo.toml")],
            cwd=root,
            check=True,
        )
    print("Generated consumer compile check passed.")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("data", type=Path, help="completed generator output directory")
    args = parser.parse_args()
    try:
        check_generated(args.data)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"generated consumer check failed: {error}\n")


if __name__ == "__main__":
    main()
