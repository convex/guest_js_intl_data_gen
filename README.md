# guest_js_intl_data_gen

> [!NOTE]
> This repo is largely AI-generated and not particularly useful right now.

## License

The generator is licensed under the [Convex Functional Source License,
Version 1.1, Apache 2.0 Future License](LICENSE) (`FSL-1.1-Apache-2.0`).
Generated ICU data retains its upstream licensing; see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md),
[LICENSE-UNICODE](LICENSE-UNICODE), and [LICENSE-TZDB](LICENSE-TZDB).

## Generate

Requirements: rustup, a native C toolchain for dependencies, network access to
crates.io and the upstream ICU/Unicode data sources, Python 3.11+ for packaging

```sh
cargo test
cargo run --release -- data
```

### Overrides

- `ICU4X_LOCALES`: comma-separated locale tags, with surrounding whitespace
  trimmed. Unset means all locales at CLDR's **modern** coverage level. Empty or
  malformed entries are errors. Requested locale families include ancestors and
  descendants. `en-US` is always requested as a single locale, even when an
  override excludes English; an explicit `en-US` entry also uses this single-locale
  policy.

For a smaller, custom generation:

```sh
ICU4X_LOCALES=en,fr cargo run --release -- data
```

## Output contract

These outputs must come from the same successful generation:

- `locales.rs`: defines `pub const AVAILABLE_LOCALES: &[&str]`, a sorted,
  deduplicated list suitable for `binary_search`. Availability includes explicit
  requests with inherited data, supported descendants, and selected ancestors,
  using each marker's fallback rules before payload deduplication. Shared ICU
  identifiers without a language (`und` and `und-*`) are not public locale
  availability; `en-US` is explicitly included.
- `icu/`: the ICU4X baked exporter tree, including `mod.rs` and its generated
  Rust files/macros. Preserve the entire tree and its relative paths. Generated
  references name the individual ICU component crates, rather than the `icu`
  metacrate. Consumers must supply compatible ICU4X dependency versions/features.
- `generation.json`: packaging metadata containing the source provider's tested
  tags, the generation-time locale override, and sorted effective locale requests.
  It is written only after a successful export and embedded in the release manifest;
  consumers need only the two Rust outputs above.

`src/main.rs::markers()` is the authoritative curated marker list. It covers
collation, NFD normalization, case mapping, calendars/date/time, decimal/currency/
percent/units, display names, relative time, lists, plurals, segmentation, locale
aliases/likely subtags, and fallback parents. Generation uses:

- requested locale families with descendants, plus a single `en-US` request to
  preserve its verbatim resolution;
- maximal fallback deduplication;
- an empty root (`und`) entry for each display-name marker, which CLDR's root
  lacks, so a locale without names of its own falls back to "no names" instead
  of missing data;
- the recommended segmentation models and additional `search*` collations;
- baked-exporter defaults, with overwrite and separate component crates enabled.

Repeated generation replaces the outputs. The update is **not atomic**: a failed
run can leave partial output. Previous `generation.json` metadata is removed at
startup so a failed rerun cannot be packaged with stale provenance. Prefer a
fresh output directory and package it only after the generator exits successfully.
The packager requires valid generation metadata and the Rust entry points, but
does not verify the completeness of an ICU export.

## Package and verify

Run these from the same clean generator checkout used for generation. `--commit`
takes its full 40-character commit SHA. The script does not inspect Git state;
the workflow supplies the checked-out commit automatically.
Use a new destination directory for each packaging attempt.

```sh
python3 -m unittest discover -s scripts -p 'test_*.py' -v
cargo fetch --locked
python3 scripts/check_generated.py data
python3 scripts/package.py data dist --commit FULL_40_CHARACTER_COMMIT_SHA
(cd dist && sha256sum -c SHA256SUMS)
tar -tzf dist/guest_js_intl_data.tar.gz
```

The archive is named **`guest_js_intl_data.tar.gz`**, with these paths at its
root (no enclosing `data/` directory):

```text
locales.rs
icu/mod.rs
icu/...
LICENSE
LICENSE-UNICODE
LICENSE-TZDB
THIRD_PARTY_NOTICES.md
manifest.json
```

`SHA256SUMS` hashes the compressed archive. `manifest.json` records the generator
repository/commit, toolchain, input source hashes, provider version, and checksums
of the archived data and license files. Its `generation` field embeds the metadata
from `generation.json`, including source tags and locale selection. Export settings
and marker selection are defined by the generator source at that commit, not
independently maintained by the packager. The packager also writes
`RELEASE_NOTES.md` for the release body.
It refuses symlinks/special files in the data and refuses to overwrite an
existing archive.

Tar entries are lexically sorted regular files with mode `0644`, zero timestamps
and uid/gid, and empty owner/group names. Gzip uses level 9, a zero timestamp,
and no original filename. Identical inputs yield identical archives with the
same Python/zlib implementation, regardless of paths, filesystem timestamps,
permissions, or ownership. Empty directories are omitted.

## Reproducibility and regeneration

Keep these inputs together when reproducing a release:

1. The generator commit, including the marker selection and export logic.
2. `Cargo.toml` and the checked-in `Cargo.lock`. Direct dependencies are pinned
   exactly; the lockfile pins transitive versions and checksums. Use `--locked`;
   do not regenerate the lockfile just to run the generator.
3. `rust-toolchain.toml`, including rustfmt used by the exporter when applicable.
4. `ICU4X_LOCALES` (unset for standard releases) and the actual source data.
   The pinned source provider selects its tested tags, recorded automatically
   in `generation.json` and documented in
   [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
5. The packaging script and Python/zlib implementation; the workflow uses
   `ubuntu-24.04`. Runner images and compression implementations can change.

The provider's downloader assembles its own input layout, including the Unicode
UCD/security tree. Upstream tags and cache contents are external inputs, **not
content-addressed by Cargo.lock**. Exact byte-for-byte regeneration requires the
same downloaded bytes; retain and independently hash a trusted source cache
when stronger reproducibility is required. The manifest records source tags,
not checksums of all downloaded archives. A cache location itself is not a
semantic input. No claim of cross-toolchain, cross-platform, or cross-runner
bit-for-bit generated source is made.

When changing generation behavior or ICU versions, review the curated markers,
update exact dependency pins and `Cargo.lock` deliberately, and review upstream
license notices. Update the source tags in `THIRD_PARTY_NOTICES.md`; a Rust test
checks them against the provider's constants. Regenerate into a fresh directory,
compile the generated provider, compare locale lists, marker reports and output
hashes, and publish a new tag.

## Validation and releases

Local tests do not download source data or run generation:

```sh
cargo fmt --check
cargo test --locked
python3 -m unittest discover -s scripts -p 'test_*.py' -v
```

The Rust tests cover CLI parsing, cache defaults, locale selection and fallback,
sorted source emission, curated marker invariants, and generation provenance.
Python tests cover the archive/provenance contract, deterministic packaging,
common packaging mistakes, and the compile check's dependency selection.

Branch pushes and pull requests run these lightweight checks. Tag/manual runs
also perform full generation and run `scripts/check_generated.py`: it expands
all baked provider implementations and compiles `locales.rs` in a temporary
consumer, using the generator's resolved dependency versions/features. Only the
consumer's temporary copy of `Cargo.lock` is reconciled; dependency version,
source, or checksum changes are rejected. The repository lockfile is unchanged.
This is a compile smoke test, not a substitute for the consumer's runtime tests.

Maintainers of `convex/guest_js_intl_data_gen` can:

1. Commit the reviewed generator, lockfile, toolchain, license notices, and tests.
   Keep generated data, build caches, and unrelated repository files out of Git.
2. Run **Generate ICU4X data** via `workflow_dispatch` on the reviewed ref.
   Inspect its validation logs and downloadable `guest-js-intl-data` artifact.
   Manual dispatch never publishes a release, including dispatch on a tag.
3. Create and push a new `v*` tag pointing to the reviewed commit. The tag push
   runs `cargo test --locked`, packaging tests, and
   `cargo run --release --locked -- data`, then compiles the generated provider,
   packages, and verifies the output.
4. The tag-only release job uses `gh release create --verify-tag` to publish
   `guest_js_intl_data.tar.gz` and `SHA256SUMS`. Its notes identify the generator
   commit and generation inputs. The archive carries the more detailed manifest.

The workflow defaults to read-only repository permissions; only the tag-push
release job has `contents: write`. Checkout does not persist credentials, and
all third-party actions are pinned by commit SHA. Existing releases cause an
error; assets are never replaced with `--clobber`. Rerun a failure only if no
release was created. If a release exists (even if incomplete), investigate and
use a new tag rather than overwriting published assets. Enable GitHub release
immutability in repository settings if available for additional protection.
