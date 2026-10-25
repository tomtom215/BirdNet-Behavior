//! The mapping table in `docs/book/guides/migration.md` is the importer's table.
//!
//! The guide used to point at "the mapping table in Configuration", which did
//! not exist, and name BirdNET-Pi keys (`ALSA_CARD`, `BIRDWEATHER_TOKEN`) that
//! BirdNET-Pi does not write. A table written by hand drifts the first time a
//! rule changes, so this one is generated from [`birdnet_migrate::conf::RULES`]
//! and this test fails when the page and the importer differ.
//!
//! To update the page after changing the table:
//!
//! ```text
//! UPDATE_MIGRATION_GUIDE=1 cargo test -p birdnet-migrate --test the_migration_guide_carries_the_mapping_table
//! ```

use std::path::PathBuf;

use birdnet_migrate::conf::mapping_table_markdown;

const START: &str = "<!-- mapping-table:start -->";
const END: &str = "<!-- mapping-table:end -->";

fn guide_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/book/guides/migration.md")
}

fn between(text: &str) -> (usize, usize) {
    let start = text.find(START).expect("the guide has the start marker") + START.len();
    let end = text.find(END).expect("the guide has the end marker");
    assert!(start <= end, "the markers are the wrong way round");
    (start, end)
}

#[test]
fn the_page_carries_exactly_the_importers_table() {
    let path = guide_path();
    let text = std::fs::read_to_string(&path).expect("the migration guide");
    let (start, end) = between(&text);
    let wanted = format!("\n{}", mapping_table_markdown());
    if std::env::var_os("UPDATE_MIGRATION_GUIDE").is_some() {
        let updated = format!("{}{}{}", &text[..start], wanted, &text[end..]);
        std::fs::write(&path, updated).expect("write the guide");
        return;
    }
    assert_eq!(
        text[start..end].trim(),
        wanted.trim(),
        "docs/book/guides/migration.md differs from the importer's table; regenerate it with \
         UPDATE_MIGRATION_GUIDE=1 (see the top of this file)"
    );
}

/// The page says nothing about the importer that the table does not: every key
/// it names in backticks as BirdNET-Pi's is one the table has.
#[test]
fn every_birdnet_pi_key_the_page_names_is_in_the_table() {
    let text = std::fs::read_to_string(guide_path()).expect("the migration guide");
    let table = mapping_table_markdown();
    for key in [
        "BIRDWEATHER_ID",
        "PURGE_THRESHOLD",
        "FULL_DISK",
        "REC_CARD",
        "RTSP_STREAM",
        "PRIVACY_THRESHOLD",
        "MAX_FILES_SPECIES",
        "AUDIOFMT",
        "DATA_MODEL_VERSION",
        "CADDY_PWD",
        "APPRISE_ONLY_NOTIFY_SPECIES_NAMES",
    ] {
        assert!(text.contains(key), "the page does not mention {key}");
        assert!(
            table.contains(&format!("`{key}`")),
            "the table has no row for {key}"
        );
    }
    // The names the page used to claim BirdNET-Pi wrote, and does not.
    for not_birdnet_pi in ["ALSA_CARD", "BIRDWEATHER_TOKEN"] {
        assert!(
            !table.contains(&format!("| `{not_birdnet_pi}`")),
            "{not_birdnet_pi} is this station's key, not BirdNET-Pi's, and has no row of its own"
        );
    }
}
