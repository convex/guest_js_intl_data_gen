//! Generates baked ICU4X data into an explicitly supplied output directory.
//!
//! Sources are pinned to the tags `icu_provider_source` was tested against,
//! which are also the tags the published `icu_*_data` crates record in
//! `[package.metadata.sources]`, so one source set serves every component.
//! They are fetched through `icu_provider_source`'s own downloader rather than
//! assembled by hand: enumerated properties are read out of a UCD tree laid out
//! per UAX #44, which is more than a couple of archives to place.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    env,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use icu_provider::{DataMarker, IterableDynamicDataProvider};
use icu_provider_export::{ExportMetadata, baked_exporter, prelude::*};
use icu_provider_source::{CoverageLevel, SourceDataProvider};

const USAGE: &str = "Usage: guest_js_intl_data_gen [--] OUTPUT_DIR
       guest_js_intl_data_gen --help

Writes locales.rs, icu/, and generation.json into OUTPUT_DIR, creating it if needed.
ICU4X_LOCALES: comma-separated locale tags (default: CLDR modern coverage).
ICU4X_SOURCE_CACHE: download cache (default: /tmp/icu4x-source-cache).";

fn output_directory(args: impl IntoIterator<Item = OsString>) -> anyhow::Result<Option<PathBuf>> {
    let mut args = args.into_iter();
    let first = args.next().context(USAGE)?;
    if first == "--help" || first == "-h" {
        anyhow::ensure!(args.next().is_none(), USAGE);
        return Ok(None);
    }
    let path = if first == "--" {
        args.next().context(USAGE)?
    } else {
        anyhow::ensure!(!first.to_string_lossy().starts_with('-'), USAGE);
        first
    };
    anyhow::ensure!(!path.is_empty() && args.next().is_none(), USAGE);
    Ok(Some(PathBuf::from(path)))
}

fn main() -> anyhow::Result<()> {
    let Some(out_dir) = output_directory(env::args_os().skip(1))? else {
        println!("{USAGE}");
        return Ok(());
    };
    fs::create_dir_all(&out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;
    invalidate_generation(&out_dir)?;
    let locale_override = match env::var("ICU4X_LOCALES") {
        Ok(value) => Some(value),
        Err(env::VarError::NotPresent) => None,
        Err(error) => return Err(error).context("ICU4X_LOCALES must be valid Unicode"),
    };
    let provider = SourceDataProvider::new();

    let fallbacker = LocaleFallbacker::try_new_unstable(&provider)
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    let locales = locales(&provider, locale_override.as_deref())?;
    let requests = locale_requests(&locales);
    let markers = markers();
    let exported = exported_locales(&provider, &fallbacker, &requests, &markers)?;
    eprintln!(
        "icu data locales: {} requested, {} exported",
        locales.len(),
        exported.len(),
    );
    let families = requests.iter().map(|(&locale, &include_family)| {
        if include_family {
            DataLocaleFamily::with_descendants(locale)
        } else {
            DataLocaleFamily::single(locale)
        }
    });

    let mut exporter_options = baked_exporter::Options::default();
    // Repeated generation into the same output directory replaces the baked data.
    exporter_options.overwrite = true;
    // Types are named through the component crates; the `icu` metacrate is not
    // a dependency of this crate.
    exporter_options.use_separate_crates = true;
    let exporter = baked_exporter::BakedExporter::new(out_dir.join("icu"), exporter_options)
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    let metadata = ExportDriver::new(
        families,
        FallbackOptions::from(DeduplicationStrategy::Maximal),
        fallbacker,
    )
    .with_markers(markers)
    .with_additional_collations([String::from("search*")])
    .export(&provider, exporter)
    .map_err(|error| anyhow::anyhow!("{error:?}"))?;

    report(&metadata);
    fs::write(out_dir.join("locales.rs"), available_locales(&exported))?;
    fs::write(
        out_dir.join("generation.json"),
        serde_json::to_vec_pretty(&generation_metadata(locale_override.as_deref(), &locales))?,
    )?;
    Ok(())
}

// Removing the success metadata first prevents packaging a failed rerun with stale provenance.
fn invalidate_generation(out_dir: &Path) -> anyhow::Result<()> {
    match fs::remove_file(out_dir.join("generation.json")) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("failed to remove previous generation metadata"),
    }
}

fn generation_metadata(locale_override: Option<&str>, locales: &[DataLocale]) -> serde_json::Value {
    let requested: BTreeSet<_> = locale_requests(locales)
        .keys()
        .map(DataLocale::to_string)
        .collect();
    serde_json::json!({
        "format_version": 1,
        "source_tags": {
            "cldr": SourceDataProvider::TESTED_CLDR_TAG,
            "icuexport": SourceDataProvider::TESTED_ICUEXPORT_TAG,
            "segmenter_lstm": SourceDataProvider::TESTED_SEGMENTER_LSTM_TAG,
            "unicode": SourceDataProvider::TESTED_UNICODE_TAG,
            "tzdb": SourceDataProvider::TESTED_TZDB_TAG,
        },
        "icu4x_locales_override": locale_override,
        "requested_locales": requested,
    })
}

/// Markers to bake. Curated per component rather than taken wholesale: in
/// particular `icu_experimental::MARKERS` would pull the transliterator and
/// person-name data, which ECMA-402 never reads.
fn markers() -> Vec<DataMarkerInfo> {
    [
        icu_collator::provider::MARKERS.to_vec(),
        // The collator normalizes to NFD as it compares; the other
        // normalization forms are not reached from anything here.
        vec![
            icu_normalizer::provider::NormalizerNfdDataV1::INFO,
            icu_normalizer::provider::NormalizerNfdTablesV1::INFO,
        ],
        // Only the case-mapping table. `CaseMapUnfoldV1` serves case folding,
        // which ECMA-402 has no operation for.
        vec![icu_casemap::provider::CaseMapV1::INFO],
        icu_calendar::provider::MARKERS.to_vec(),
        icu_datetime::provider::MARKERS.to_vec(),
        icu_decimal::provider::MARKERS.to_vec(),
        icu_time::provider::MARKERS.to_vec(),
        // The dimensions, display names, and relative-time data ECMA-402 uses,
        // without the experimental crate's transliteration or person-name data.
        vec![
            icu_experimental::dimension::provider::currency::essentials::CurrencyEssentialsV1::INFO,
            icu_experimental::dimension::provider::currency::extended::CurrencyExtendedDataV1::INFO,
            icu_experimental::dimension::provider::currency::fractions::CurrencyFractionsV1::INFO,
            icu_experimental::dimension::provider::currency::patterns::CurrencyPatternsDataV1::INFO,
            icu_experimental::dimension::provider::currency::symbols::CurrencySymbolsV1::INFO,
            icu_experimental::dimension::provider::percent::PercentEssentialsV1::INFO,
            icu_experimental::dimension::provider::units::essentials::UnitsEssentialsV1::INFO,
            icu_experimental::displaynames::provider::LanguageDisplayNamesV1::INFO,
            icu_experimental::displaynames::provider::LocaleDisplayNamesV1::INFO,
            icu_experimental::displaynames::provider::RegionDisplayNamesV1::INFO,
            icu_experimental::displaynames::provider::ScriptDisplayNamesV1::INFO,
            icu_experimental::displaynames::provider::VariantDisplayNamesV1::INFO,
            icu_experimental::relativetime::provider::LongSecondRelativeV1::INFO,
            icu_experimental::relativetime::provider::LongMinuteRelativeV1::INFO,
            icu_experimental::relativetime::provider::LongHourRelativeV1::INFO,
            icu_experimental::relativetime::provider::LongDayRelativeV1::INFO,
            icu_experimental::relativetime::provider::LongWeekRelativeV1::INFO,
            icu_experimental::relativetime::provider::LongMonthRelativeV1::INFO,
            icu_experimental::relativetime::provider::LongQuarterRelativeV1::INFO,
            icu_experimental::relativetime::provider::LongYearRelativeV1::INFO,
            icu_experimental::relativetime::provider::ShortSecondRelativeV1::INFO,
            icu_experimental::relativetime::provider::ShortMinuteRelativeV1::INFO,
            icu_experimental::relativetime::provider::ShortHourRelativeV1::INFO,
            icu_experimental::relativetime::provider::ShortDayRelativeV1::INFO,
            icu_experimental::relativetime::provider::ShortWeekRelativeV1::INFO,
            icu_experimental::relativetime::provider::ShortMonthRelativeV1::INFO,
            icu_experimental::relativetime::provider::ShortQuarterRelativeV1::INFO,
            icu_experimental::relativetime::provider::ShortYearRelativeV1::INFO,
            icu_experimental::relativetime::provider::NarrowSecondRelativeV1::INFO,
            icu_experimental::relativetime::provider::NarrowMinuteRelativeV1::INFO,
            icu_experimental::relativetime::provider::NarrowHourRelativeV1::INFO,
            icu_experimental::relativetime::provider::NarrowDayRelativeV1::INFO,
            icu_experimental::relativetime::provider::NarrowWeekRelativeV1::INFO,
            icu_experimental::relativetime::provider::NarrowMonthRelativeV1::INFO,
            icu_experimental::relativetime::provider::NarrowQuarterRelativeV1::INFO,
            icu_experimental::relativetime::provider::NarrowYearRelativeV1::INFO,
        ],
        // Unit patterns, read by `Intl.NumberFormat`'s `unit` style and by
        // `Intl.DurationFormat`. ICU4X groups them by measurement category and
        // then by how widely a locale uses them, and a unit's tier varies from
        // locale to locale, so all three tiers of all five categories are
        // needed to cover one unit everywhere.
        {
            use icu_experimental::dimension::provider::units::categorized_display_names::*;
            vec![
                UnitsNamesAreaCoreV1::INFO,
                UnitsNamesAreaExtendedV1::INFO,
                UnitsNamesAreaOutlierV1::INFO,
                UnitsNamesDurationCoreV1::INFO,
                UnitsNamesDurationExtendedV1::INFO,
                UnitsNamesDurationOutlierV1::INFO,
                UnitsNamesLengthCoreV1::INFO,
                UnitsNamesLengthExtendedV1::INFO,
                UnitsNamesLengthOutlierV1::INFO,
                UnitsNamesMassCoreV1::INFO,
                UnitsNamesMassExtendedV1::INFO,
                UnitsNamesMassOutlierV1::INFO,
                UnitsNamesVolumeCoreV1::INFO,
                UnitsNamesVolumeExtendedV1::INFO,
                UnitsNamesVolumeOutlierV1::INFO,
            ]
        },
        icu_list::provider::MARKERS.to_vec(),
        icu_plurals::provider::MARKERS.to_vec(),
        icu_segmenter::provider::MARKERS.to_vec(),
        // Only what `LocaleCanonicalizer` and `LocaleExpander` read. The
        // crate's own `MARKERS` also carries display names and exemplar
        // characters, whose data dwarfs this and which nothing loads yet.
        vec![
            icu_locale::provider::LocaleAliasesV1::INFO,
            icu_locale::provider::LocaleLikelySubtagsExtendedV1::INFO,
            icu_locale::provider::LocaleLikelySubtagsScriptRegionV1::INFO,
            icu_locale_fallback::provider::LocaleLikelySubtagsLanguageV1::INFO,
            // Read by `LocaleFallbacker`, which drives the best-fit matcher.
            icu_locale_fallback::provider::LocaleParentsV1::INFO,
        ],
    ]
    .concat()
}

fn parse_locales(list: &OsStr) -> anyhow::Result<Vec<DataLocale>> {
    list.to_string_lossy()
        .split(',')
        .map(|tag| {
            tag.trim()
                .parse::<DataLocale>()
                .with_context(|| format!("ICU4X_LOCALES contains an invalid tag: {tag}"))
        })
        .collect()
}

fn locales(
    provider: &SourceDataProvider,
    locale_override: Option<&str>,
) -> anyhow::Result<Vec<DataLocale>> {
    if let Some(list) = locale_override {
        return parse_locales(OsStr::new(list));
    }
    // All locales reaching CLDR's "modern" coverage level
    Ok(provider
        .locales_for_coverage_levels([CoverageLevel::Modern])
        .map_err(|error| anyhow::anyhow!(error))?
        .into_iter()
        .collect())
}

/// Whether each request includes ancestors and descendants, or just the single locale.
/// Share the effective requests with the exporter so duplicate requests have one meaning.
fn locale_requests(locales: &[DataLocale]) -> HashMap<DataLocale, bool> {
    let mut requests: HashMap<_, _> = locales
        .iter()
        .map(|&locale| (locale, !locale.is_unknown()))
        .collect();
    // test262 assumes `en-US` resolves verbatim, even when English wasn't requested.
    requests.insert(data_locale!("en-US"), false);
    requests
}

/// Public language locales selected before payload deduplication. ICU's `und` and
/// `und-*` identifiers are shared/region/script data, not JavaScript language availability.
fn exported_locales(
    provider: &SourceDataProvider,
    fallbacker: &LocaleFallbacker,
    requests: &HashMap<DataLocale, bool>,
    markers: &[DataMarkerInfo],
) -> anyhow::Result<BTreeSet<String>> {
    let mut exported = BTreeSet::new();
    for marker in markers.iter().filter(|marker| !marker.is_singleton) {
        let supported = provider
            .iter_ids_for_marker(*marker)
            .map_err(|error| anyhow::anyhow!("{error}"))?
            .into_iter()
            // The driver does not inherit attributed root IDs (e.g. segmentation models).
            .filter(|id| !id.locale.is_unknown() || id.is_unknown())
            .map(|id| id.locale)
            .collect();
        exported.extend(selected_locales(
            supported,
            &fallbacker.for_config(marker.fallback_config),
            requests,
        ));
    }
    exported.insert(String::from("en-US"));
    Ok(exported)
}

fn selected_locales(
    supported: HashSet<DataLocale>,
    fallbacker: &icu_locale_fallback::LocaleFallbackerWithConfig<'_>,
    requests: &HashMap<DataLocale, bool>,
) -> BTreeSet<String> {
    // Requested locales can inherit data even when the provider doesn't enumerate them.
    let candidates: HashSet<_> = supported.iter().chain(requests.keys()).copied().collect();
    let mut selected: HashSet<_> = requests.keys().copied().collect();
    let mut with_data = HashSet::new();
    for &locale in &candidates {
        let include_ancestors = requests.get(&locale).copied().unwrap_or(false);
        let mut has_data = supported.contains(&locale);
        let mut iter = fallbacker.fallback_for(locale);
        loop {
            let parent = *iter.get();
            has_data |= supported.contains(&parent);
            if include_ancestors {
                selected.insert(parent);
            }
            if requests.get(&parent) == Some(&true) {
                selected.insert(locale);
            }
            if parent.is_unknown() {
                break;
            }
            iter.step();
        }
        if has_data {
            with_data.insert(locale);
        }
    }
    selected
        .intersection(&with_data)
        .filter(|locale| !locale.language.is_unknown())
        .map(DataLocale::to_string)
        .collect()
}

/// Emits [`exported_locales`] as source. This is the single
/// `[[AvailableLocales]]` for every service: deriving it from the export rather
/// than probing markers at runtime keeps it independent of the deduplication
/// strategy, which drops locales whose payload matches their fallback parent.
fn available_locales(exported: &BTreeSet<String>) -> String {
    let mut out = String::from(
        "/// Locales for which data was baked, sorted, for `binary_search`.\npub const \
         AVAILABLE_LOCALES: &[&str] = &[\n",
    );
    for tag in exported {
        out.push_str(&format!("    {tag:?},\n"));
    }
    out.push_str("];\n");
    out
}

/// Reports per-marker baked sizes, so the data budget is visible in build logs
/// rather than needing a separate measurement pass.
fn report(metadata: &ExportMetadata) {
    let Some(baked) = metadata
        .exporter
        .0
        .as_ref()
        .and_then(|any| any.downcast_ref::<baked_exporter::BakedExporterCloseMetadata>())
    else {
        return;
    };
    let mut total = 0;
    for (marker, statistics) in &baked.statistics {
        let size = statistics.structs_total_size + statistics.lookup_struct_size;
        total += size;
        eprintln!(
            "icu data {marker:?}: {size} B ({} structs, {} identifiers)",
            statistics.structs_count, statistics.identifiers_count,
        );
    }
    eprintln!("icu data total: {total} B");
    eprintln!(
        "icu data required crates: {}",
        baked
            .required_crates
            .iter()
            .copied()
            .collect::<Vec<_>>()
            .join(", "),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> anyhow::Result<Option<PathBuf>> {
        output_directory(args.iter().map(OsString::from))
    }

    #[test]
    fn cli_usage() {
        assert_eq!(cli(&["data"]).unwrap(), Some(PathBuf::from("data")));
        assert_eq!(cli(&["--help"]).unwrap(), None);
        assert_eq!(
            cli(&["--", "--data"]).unwrap(),
            Some(PathBuf::from("--data"))
        );
        for args in [&[][..], &["a", "b"][..], &["--unknown"][..]] {
            assert!(cli(args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn locale_override_trims_whitespace_and_accepts_duplicates() {
        let parsed = parse_locales(OsStr::new(" fr, en-US,fr ")).unwrap();
        assert_eq!(
            locale_requests(&parsed),
            HashMap::from([(data_locale!("fr"), true), (data_locale!("en-US"), false)])
        );
    }

    #[test]
    fn locale_override_rejects_invalid_and_empty_tags() {
        for value in ["", "en,", "en,,fr", "en,not_a_locale!"] {
            assert!(
                parse_locales(OsStr::new(value))
                    .unwrap_err()
                    .to_string()
                    .contains("ICU4X_LOCALES")
            );
        }
    }

    #[test]
    fn available_locales_is_sorted_rust_source() {
        let locales = ["fr", "en-US", "en", "fr"]
            .map(String::from)
            .into_iter()
            .collect();
        assert_eq!(
            available_locales(&locales),
            concat!(
                "/// Locales for which data was baked, sorted, for `binary_search`.\n",
                "pub const AVAILABLE_LOCALES: &[&str] = &[\n",
                "    \"en\",\n",
                "    \"en-US\",\n",
                "    \"fr\",\n",
                "];\n",
            )
        );
    }

    #[test]
    fn marker_selection_is_curated_and_unique() {
        let markers = markers();
        let names: BTreeSet<_> = markers.iter().map(|marker| format!("{marker:?}")).collect();
        assert_eq!(names.len(), markers.len());
        assert!(markers.contains(&icu_normalizer::provider::NormalizerNfdDataV1::INFO));
        assert!(markers.contains(&icu_casemap::provider::CaseMapV1::INFO));
        assert!(markers.contains(&icu_locale_fallback::provider::LocaleParentsV1::INFO));
        assert!(!markers.contains(&icu_casemap::provider::CaseMapUnfoldV1::INFO));
    }

    #[test]
    fn generation_metadata_captures_inputs() {
        let metadata =
            generation_metadata(Some(" fr,fr "), &[data_locale!("fr"), data_locale!("fr")]);
        assert_eq!(metadata["format_version"], 1);
        assert_eq!(metadata["icu4x_locales_override"], " fr,fr ");
        assert_eq!(
            metadata["requested_locales"],
            serde_json::json!(["en-US", "fr"])
        );
        assert!(generation_metadata(None, &[])["icu4x_locales_override"].is_null());
        for tag in metadata["source_tags"].as_object().unwrap().values() {
            let tag = tag.as_str().unwrap();
            assert!(
                include_str!("../THIRD_PARTY_NOTICES.md").contains(&format!("`{tag}`")),
                "source tag {tag} is missing from the license provenance"
            );
        }
    }

    struct FallbackData;

    impl icu_provider::DataProvider<icu_locale_fallback::provider::LocaleLikelySubtagsLanguageV1>
        for FallbackData
    {
        fn load(
            &self,
            _: icu_provider::DataRequest,
        ) -> Result<
            icu_provider::DataResponse<
                icu_locale_fallback::provider::LocaleLikelySubtagsLanguageV1,
            >,
            icu_provider::DataError,
        > {
            let en = locale!("en-Latn-US").id;
            Ok(icu_provider::DataResponse {
                metadata: Default::default(),
                payload: icu_provider::DataPayload::from_owned(
                    icu_locale_fallback::provider::LikelySubtagsForLanguage {
                        language: [(
                            en.language.to_tinystr().to_unvalidated(),
                            (en.script.unwrap(), en.region.unwrap()),
                        )]
                        .into_iter()
                        .collect(),
                        language_script: Default::default(),
                        language_region: Default::default(),
                        und: (en.language, en.script.unwrap(), en.region.unwrap()),
                    },
                ),
            })
        }
    }

    impl icu_provider::DataProvider<icu_locale_fallback::provider::LocaleParentsV1> for FallbackData {
        fn load(
            &self,
            _: icu_provider::DataRequest,
        ) -> Result<
            icu_provider::DataResponse<icu_locale_fallback::provider::LocaleParentsV1>,
            icu_provider::DataError,
        > {
            Ok(icu_provider::DataResponse {
                metadata: Default::default(),
                payload: icu_provider::DataPayload::from_owned(Default::default()),
            })
        }
    }

    fn selection(
        requested: &[&str],
        supported: &[&str],
        priority: icu_locale_fallback::LocaleFallbackPriority,
    ) -> BTreeSet<String> {
        let fallbacker = LocaleFallbacker::try_new_unstable(&FallbackData).unwrap();
        let mut config = icu_locale_fallback::LocaleFallbackConfig::default();
        config.priority = priority;
        let requested: Vec<_> = requested.iter().map(|tag| tag.parse().unwrap()).collect();
        selected_locales(
            supported.iter().map(|tag| tag.parse().unwrap()).collect(),
            &fallbacker.for_config(config),
            &locale_requests(&requested),
        )
    }

    #[test]
    fn requested_locale_with_implied_script_inherits_data() {
        assert_eq!(
            selection(
                &["en-Latn"],
                &["en", "en-GB"],
                icu_locale_fallback::LocaleFallbackPriority::Language
            ),
            BTreeSet::from(["en".into(), "en-Latn".into(), "en-US".into()]),
        );
    }

    #[test]
    fn regional_request_includes_ancestors_but_not_siblings() {
        assert_eq!(
            selection(
                &["en-GB"],
                &["en", "en-GB", "en-AU"],
                icu_locale_fallback::LocaleFallbackPriority::Language
            ),
            BTreeSet::from(["en".into(), "en-GB".into(), "en-US".into()]),
        );
    }

    #[test]
    fn language_request_includes_supported_descendants() {
        assert_eq!(
            selection(
                &["en"],
                &["en", "en-GB", "fr"],
                icu_locale_fallback::LocaleFallbackPriority::Language
            ),
            BTreeSet::from(["en".into(), "en-GB".into(), "en-US".into()]),
        );
    }

    #[test]
    fn root_and_single_en_us_do_not_select_other_families() {
        assert_eq!(
            selection(
                &["und", "en-US"],
                &["und", "und-US", "en", "en-GB", "fr"],
                icu_locale_fallback::LocaleFallbackPriority::Language
            ),
            BTreeSet::from(["en-US".into()]),
        );
    }

    #[test]
    fn requested_locale_needs_inheritable_data() {
        assert!(
            selection(
                &["fr"],
                &["en-GB"],
                icu_locale_fallback::LocaleFallbackPriority::Language
            )
            .is_empty()
        );
    }

    #[test]
    fn selection_uses_marker_fallback_priority() {
        let supported = ["en", "en-GB", "und-GB", "und"];
        let language = selection(
            &["en-GB"],
            &supported,
            icu_locale_fallback::LocaleFallbackPriority::Language,
        );
        let region = selection(
            &["en-GB"],
            &supported,
            icu_locale_fallback::LocaleFallbackPriority::Region,
        );
        assert!(language.contains("en"));
        assert!(!region.contains("en"));
        assert!(region.contains("en-GB"));
        assert!(!region.contains("und-GB"));
    }
}
