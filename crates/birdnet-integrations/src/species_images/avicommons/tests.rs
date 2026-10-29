//! Gates for the AviCommons provider. Nothing here touches the network: the
//! licence filter is the whole of what keeps an image the station may not show
//! out of a species page, and the index handling is what keeps a Pi from asking
//! AviCommons for 1.6 MB on every start.

use super::*;

/// Every distinct licence string in AviCommons' `latest.json` as downloaded on
/// 2026-10-21 (41 spellings over 10,185 images), with whether it permits
/// commercial use, decided by reading each one.
const REAL_LICENCES: [(&str, bool); 41] = [
    ("CC BY 1.0", true),
    ("CC BY 2.0", true),
    ("CC BY 2.5", true),
    ("CC BY 3.0", true),
    ("CC BY 3.0-de", true),
    ("CC BY 3.0-us", true),
    ("CC BY 4.0", true),
    ("CC BY-NC 1.0", false),
    ("CC BY-NC 2.0", false),
    ("CC BY-NC 2.0-de", false),
    ("CC BY-NC 2.5", false),
    ("CC BY-NC 2.5-br", false),
    ("CC BY-NC 2.5-in", false),
    ("CC BY-NC 2.5-se", false),
    ("CC BY-NC 3.0", false),
    ("CC BY-NC 3.0-au", false),
    ("CC BY-NC 3.0-de", false),
    ("CC BY-NC 3.0-nz", false),
    ("CC BY-NC 4.0", false),
    ("CC BY-NC-SA 2.0", false),
    ("CC BY-NC-SA 2.5", false),
    ("CC BY-NC-SA 3.0", false),
    ("CC BY-NC-SA 4.0", false),
    ("CC BY-SA 2.0", true),
    ("CC BY-SA 2.5", true),
    ("CC BY-SA 2.5-in", true),
    ("CC BY-SA 3.0", true),
    ("CC BY-SA 3.0-de", true),
    ("CC BY-SA 3.0-nz", true),
    ("CC BY-SA 4.0", true),
    ("CC0 1.0", true),
    ("CC0 2.0", true),
    ("CC0 2.5", true),
    ("CC0 2.5-au", true),
    ("CC0 3.0", true),
    ("CC0 4.0", true),
    ("cc-by", true),
    ("cc-by-nc", false),
    ("cc-by-nc-sa", false),
    ("cc-by-sa", true),
    ("cc0", true),
];

#[test]
fn every_licence_spelling_in_the_real_index_is_classified_as_read() {
    for (licence, permitted) in REAL_LICENCES {
        assert_eq!(
            licence_permits_commercial_use(licence),
            permitted,
            "{licence:?}"
        );
    }
    let allowed = REAL_LICENCES.iter().filter(|(_, p)| *p).count();
    assert_eq!(allowed, 23, "23 of the 41 spellings permit commercial use");
}

/// A spelling that does not exist yet is refused, never allowed: it costs a
/// picture, not a licence.
#[test]
fn what_is_not_recognised_is_refused() {
    for odd in [
        "",
        "  ",
        "all rights reserved",
        "CC BY-ND 4.0",
        "cc-by-nd",
        "CC BY-NC-ND 4.0",
        "CC-BY 4.0 International",
        "public domain",
        "CC SA 4.0",
        "CC BY-XX-SA 4.0 weird",
        "CC BY 4.0 NC",
    ] {
        assert!(!licence_permits_commercial_use(odd), "{odd:?}");
    }
}

#[test]
fn a_credit_line_names_the_licence_the_way_people_write_it() {
    assert_eq!(licence_label("cc-by"), "CC BY");
    assert_eq!(licence_label("cc-by-sa"), "CC BY-SA");
    assert_eq!(licence_label("cc0"), "CC0");
    assert_eq!(licence_label("CC BY-SA 3.0-de"), "CC BY-SA 3.0-de");
}

fn sample_index() -> String {
    serde_json::json!([
        {"code":"houfin","name":"House Finch","sciName":"Haemorhous mexicanus","license":"CC BY 4.0","key":"111","by":"A Photographer","family":"f"},
        {"code":"eurbla","name":"Eurasian Blackbird","sciName":"Turdus merula","license":"cc-by-nc","key":"222","by":"Someone","family":"f"},
        {"code":"magpie","name":"Eurasian Magpie","sciName":"Pica pica","license":"CC BY-SA 3.0-de","key":"333","by":"  Pia  ","family":"f"},
        {"code":"nobody","name":"Uncredited Bird","sciName":"Nullus nullus","license":"cc0","key":"444","by":"  ","family":"f"},
        {"code":"houfi2","name":"House Finch dup","sciName":"Haemorhous mexicanus","license":"cc0","key":"555","by":"Second","family":"f"},
    ])
    .to_string()
}

#[test]
fn the_index_keeps_only_credited_images_the_station_may_show() {
    let index = Index::parse(&sample_index()).expect("parses");
    assert_eq!(index.total, 5);
    assert!(index.by_species.contains_key("haemorhous mexicanus"));
    assert!(index.by_species.contains_key("pica pica"));
    assert!(
        !index.by_species.contains_key("turdus merula"),
        "a NonCommercial image is not used"
    );
    assert!(
        !index.by_species.contains_key("nullus nullus"),
        "an image with no photographer to credit is not used"
    );
    // The first usable record for a species wins; the later duplicate does not
    // replace it.
    assert_eq!(index.by_species["haemorhous mexicanus"].key, "111");
    assert_eq!(
        index.by_species["pica pica"].by, "Pia",
        "the name is trimmed"
    );
}

/// A format change must read as a fault. "No images for any species, for ever"
/// is what an empty map would say.
#[test]
fn a_changed_format_is_an_error_not_an_empty_collection() {
    for bad in ["", "{}", "[]", r#"[{"code":"x"}]"#, "not json", "null"] {
        assert!(
            matches!(Index::parse(bad), Err(ImageError::Api(_))),
            "{bad:?}"
        );
    }
}

fn client(dir: &Path) -> AviCommonsClient {
    AviCommonsClient::new(dir, 320).expect("builds")
}

#[test]
fn the_image_address_is_the_documented_one_at_the_smallest_sufficient_size() {
    let dir = tempfile::tempdir().unwrap();
    let entry = Entry {
        sci_name: "Haemorhous mexicanus".into(),
        code: "houfin".into(),
        key: "111".into(),
        by: "A".into(),
        licence: "CC BY 4.0".into(),
    };
    let c = AviCommonsClient::new(dir.path(), 300).unwrap();
    assert_eq!(
        c.image_url(&entry),
        "https://static.avicommons.org/houfin-111-320.jpg"
    );
    for (want, size) in [
        (160, 160),
        (161, 240),
        (320, 320),
        (480, 480),
        (900, 900),
        (4000, 900),
    ] {
        let c = AviCommonsClient::new(dir.path(), want).unwrap();
        assert!(
            c.image_url(&entry).ends_with(&format!("-{size}.jpg")),
            "{want} -> {size}"
        );
    }
}

#[test]
fn the_credit_names_the_photographer_the_licence_and_links_back() {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::parse(&sample_index()).unwrap();
    let c = client(dir.path());
    let img = c.species_image(&index.by_species["haemorhous mexicanus"]);
    assert_eq!(
        img.credit.as_ref().map(ImageCredit::line).as_deref(),
        Some("Haemorhous mexicanus by A Photographer, AviCommons, CC BY 4.0")
    );
    assert!(
        img.description.is_none(),
        "the credit is not repeated as a description"
    );
    assert_eq!(
        img.wiki_url.as_deref(),
        Some("https://avicommons.org/species/houfin")
    );
    let credit = img.credit.expect("a structured credit");
    assert_eq!(credit.author, "A Photographer");
    assert_eq!(credit.source, "AviCommons");
    assert_eq!(credit.source_url, "https://avicommons.org/species/houfin");
    assert_eq!(credit.licence, "CC BY 4.0");
    assert_eq!(
        credit.licence_url,
        "https://creativecommons.org/licenses/by/4.0/"
    );
    assert!(credit.is_attributable());
}

#[test]
fn a_licence_links_to_its_page_only_when_the_version_is_known() {
    assert_eq!(
        licence_url("CC BY 4.0"),
        "https://creativecommons.org/licenses/by/4.0/"
    );
    assert_eq!(
        licence_url("CC BY-SA 3.0"),
        "https://creativecommons.org/licenses/by-sa/3.0/"
    );
    assert_eq!(
        licence_url("CC0"),
        "https://creativecommons.org/publicdomain/zero/1.0/"
    );
    assert_eq!(licence_url("CC BY"), "", "no version, no link");
    assert_eq!(
        licence_url("CC BY-SA 3.0-de"),
        "",
        "a regional port is not guessed at"
    );
    assert_eq!(licence_url("something else"), "");
}

/// A fresh copy on disk is used with no request: this is what keeps a station
/// that restarts daily from downloading the index daily. The base address is
/// unreachable here (`.invalid`), so any request would fail the lookup.
#[tokio::test]
async fn a_fresh_copy_on_disk_is_the_index_and_needs_no_network() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(INDEX_FILE), sample_index()).unwrap();
    let c = client(dir.path());
    let img = c
        .fetch("Haemorhous mexicanus")
        .await
        .expect("found on disk");
    assert!(img.url.contains("houfin-111-"));
    // Case-insensitive, as the species name arrives in whatever case it was typed.
    assert!(c.fetch("  PICA PICA ").await.is_ok());
    // A species with no permissive image falls through to the next provider.
    assert!(matches!(
        c.fetch("Turdus merula").await,
        Err(ImageError::NotFound(_))
    ));
    assert!(matches!(
        c.fetch("Aquila chrysaetos").await,
        Err(ImageError::NotFound(_))
    ));
}

#[test]
fn a_copy_older_than_the_limit_is_not_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(INDEX_FILE);
    std::fs::write(&p, "x").unwrap();
    assert_eq!(read_if_fresh(&p, INDEX_MAX_AGE).as_deref(), Some("x"));
    assert_eq!(read_if_fresh(&p, Duration::ZERO), None, "zero tolerance");
    assert_eq!(
        read_if_fresh(&dir.path().join("absent"), INDEX_MAX_AGE),
        None
    );
}

#[test]
fn the_index_is_written_whole_or_not_at_all() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(INDEX_FILE);
    write_atomically(&p, b"one").unwrap();
    write_atomically(&p, b"two").unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "two");
    assert!(
        !p.with_extension("json.part").exists(),
        "no part file is left behind"
    );
}
