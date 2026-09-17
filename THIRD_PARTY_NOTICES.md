# License provenance

## Generator source

The generator is licensed under the Convex Functional Source License,
Version 1.1, Apache 2.0 Future License (`FSL-1.1-Apache-2.0`).
`LICENSE` contains the same license text and Convex copyright notice used
by Convex backend crates.

## ICU4X and generated Unicode data

The generator uses ICU4X and incorporates upstream Unicode data. These materials
retain their upstream licensing independently of the generator's source license.
Published ICU4X code and data crates declare `Unicode-3.0`.

`LICENSE-UNICODE` is an unmodified copy of `LICENSE` from the crates.io package
`icu_provider_source 2.3.1`. It was also verified byte-for-byte against `LICENSE`
in the published `icu_calendar_data 2.3.0` crate, whose source metadata records the
same CLDR, ICU export, and segmentation model tags used here. The notice includes
the ICU4C/ICU4J attribution supplied by ICU4X. Its SHA-256 is:

```
f367c1b8e1aa262435251e442901da4607b4650e0e63a026f5044473ecfb90f2
```

Crate checksums for the generator's actual dependencies are in `Cargo.lock`.
Upstream package pages:

- https://crates.io/crates/icu_provider_source/2.3.1
- https://crates.io/crates/icu_calendar_data/2.3.0

Input sources selected by `icu_provider_source`:

| Input | Tag | Upstream |
| --- | --- | --- |
| CLDR JSON | `48.2.1` | https://github.com/unicode-org/cldr-json |
| ICU export | `release-78.1rc` | https://github.com/unicode-org/icu |
| Segmentation LSTM models | `v0.1.0` | https://github.com/unicode-org/lstm_word_segmentation |
| Unicode character/security data | `17.0.0` | https://www.unicode.org/Public/17.0.0/ |
| Time-zone data | `2026c` | https://www.iana.org/time-zones |

## Time-zone data

`LICENSE-TZDB` is copied verbatim from the `LICENSE` member of the cached upstream
`tzdata2026c.tar.gz` archive used by the source provider, at:
https://www.iana.org/time-zones/repository/releases/tzdata2026c.tar.gz

It states that the tz code and data are public domain except for the specific
BSD-licensed files it names. Retain this notice with the data.

## Distribution

The release archive includes `LICENSE`, `LICENSE-UNICODE`, `LICENSE-TZDB`, and
this provenance document. Keep the applicable upstream copyright and permission
notices with redistributed generated data or its accompanying documentation.
The generator's source notice does not replace upstream notices or relicense
upstream data. When updating inputs, review their licenses and update these
notices as needed. This document is provenance, not a license grant for any
third-party dependency beyond its own terms.
