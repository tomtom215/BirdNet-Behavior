//! A species keeps its colour from chart to chart (UIR-01).
//!
//! Every chart coloured its species by their rank *in that chart* (`kit::slot_class(i)`), so
//! a bird was green on Today's day strip and orange on the dawn rings, and a reader who had
//! learnt "the green one is the Robin" read the next chart wrong. The station now gives its
//! eight most-heard species of the 90 days to yesterday one colour each, and every other
//! species the neutral `ck-cother` (`kit::SpeciesSlots`).
//!
//! The fixture makes the ranks disagree on purpose: nine species heard every day for forty
//! days, the first most and the ninth least, and today the order reversed, with a tenth bird
//! heard today only. So Today's strip ranks them one way and the sixty-day dawn rings and the
//! thirty-day streamgraph and chord the other. The gate reads, from five figures served by the
//! real router, which colour class each species is drawn in, and fails if one species has two.
//!
//! The counterparts: two species never share an owned colour; the station's top eight are the
//! owners, in the order of the history (not of today); the ninth and the bird heard only today
//! are the neutral; and each figure names at least two species, so no figure passes by
//! drawing nothing.

use std::collections::{BTreeMap, BTreeSet};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// The nine historical species, most heard first, and how many times a day.
const HISTORY: [(&str, &str, i64); 9] = [
    ("American Robin", "Turdus migratorius", 18),
    ("Blue Jay", "Cyanocitta cristata", 16),
    ("Carolina Wren", "Thryothorus ludovicianus", 14),
    ("Downy Woodpecker", "Dryobates pubescens", 12),
    ("Eastern Bluebird", "Sialia sialis", 10),
    ("Fish Crow", "Corvus ossifragus", 8),
    ("Gray Catbird", "Dumetella carolinensis", 6),
    ("House Finch", "Haemorhous mexicanus", 4),
    ("Indigo Bunting", "Passerina cyanea", 2),
];
/// Heard today only.
const NEWCOMER: (&str, &str) = ("Juniper Titmouse", "Baeolophus ridgwayi");

fn seed(state: &AppState) {
    state.with_db(|conn| {
        let tx = conn.unchecked_transaction().unwrap();
        let insert = |back: i64, minute: i64, com: &str, sci: &str| {
            tx.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                 VALUES (DATE('now', 'localtime', ?1), ?2, ?3, ?4, 0.9, '')",
                rusqlite::params![
                    format!("-{back} days"),
                    format!("06:{:02}:{:02}", minute % 60, back % 60),
                    sci,
                    com
                ],
            )
            .unwrap();
        };
        // Forty days of history, all at dawn and in shared five-minute blocks, so the dawn
        // rings, the streamgraph and the co-occurrence chord all have them.
        for back in 1..=40 {
            for (com, sci, per_day) in HISTORY {
                for k in 0..per_day {
                    insert(back, k * 3, com, sci);
                }
            }
        }
        // Today the order is reversed: the least heard of the history is the most heard today.
        for (i, (com, sci, _)) in HISTORY.iter().enumerate() {
            for k in 0..=i64::try_from(i).unwrap() * 2 {
                insert(0, k * 3, com, sci);
            }
        }
        for k in 0..30 {
            insert(0, k * 2, NEWCOMER.0, NEWCOMER.1);
        }
        tx.commit().unwrap();
    });
}

async fn get(state: &AppState, uri: &str) -> String {
    let res = build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "{uri}");
    String::from_utf8(
        axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .trim()
        .to_owned()
}

/// `(name, class)` for every legend swatch: `ck-sw ck-…"></span>` then the name, directly or in
/// the dawn key's `dc-leg-name`.
fn legend_swatches(html: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for part in html.split("ck-sw ").skip(1) {
        let Some(class) = part.split('"').next() else {
            continue;
        };
        if !class.starts_with("ck-c") {
            continue;
        }
        let Some(after) = part.split_once("</span>").map(|(_, a)| a) else {
            continue;
        };
        let after = after
            .strip_prefix(r#"<span class="dc-leg-name">"#)
            .unwrap_or(after);
        let name = unescape(after.split('<').next().unwrap_or_default());
        if !name.is_empty() {
            out.push((name, class.to_owned()));
        }
    }
    out
}

/// `(name, class)` for every chord arc: its class, and the first line of its reading.
fn chord_arcs(html: &str) -> Vec<(String, String)> {
    html.split(r#"<path class="chord-arc "#)
        .skip(1)
        .filter_map(|p| {
            let class = p.split('"').next()?.to_owned();
            let tip = p.split_once(r#"data-tip=""#)?.1;
            let name = tip.split("&#10;").next()?;
            Some((unescape(name), class))
        })
        .collect()
}

/// `(name, class)` for every row of each bird's day: its square, then its name.
fn strip_rows(html: &str) -> Vec<(String, String)> {
    html.split(r#"<rect class="ck-fill "#)
        .skip(1)
        .filter_map(|p| {
            let class = p.split('"').next()?.to_owned();
            let (head, rest) = p.split_once("<text class=\"dc-strip-name\"")?;
            if head.contains('<') {
                return None;
            }
            let name = rest.split_once('>')?.1.split('<').next()?;
            Some((unescape(name), class))
        })
        .collect()
}

/// `(name, number)` for every species in Today's songs feed.
fn songs(json: &str) -> Vec<(String, u64)> {
    let v: serde_json::Value = serde_json::from_str(json).expect("the feed is JSON");
    v["species"]
        .as_array()
        .expect("species")
        .iter()
        .filter(|s| s["other"].as_bool() != Some(true))
        .map(|s| {
            (
                s["name"].as_str().unwrap_or_default().to_owned(),
                s["slot"].as_u64().unwrap_or(99),
            )
        })
        .collect()
}

#[tokio::test]
async fn one_species_one_colour_across_the_charts() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    seed(&state);

    let figures = [
        ("/pages/today-daystrip", "Today's day strip"),
        ("/pages/dawn-polar", "the dawn rings"),
        ("/pages/dawn-list", "each bird's day"),
        ("/pages/activity-streamgraph?days=30", "the streamgraph"),
        ("/pages/acoustic-network?days=30", "the chord"),
    ];
    let mut seen: BTreeMap<String, BTreeMap<String, Vec<&str>>> = BTreeMap::new();
    let mut first_named: Vec<(&str, String)> = Vec::new();
    for (uri, what) in figures {
        let html = get(&state, uri).await;
        let mut found = legend_swatches(&html);
        found.extend(chord_arcs(&html));
        found.extend(strip_rows(&html));
        let names: BTreeSet<&str> = found.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            names.len() >= 2,
            "{what} ({uri}) names {} species: the gate would grade nothing there: {html}",
            names.len()
        );
        if let Some((n, _)) = found.first() {
            first_named.push((what, n.clone()));
        }
        for (name, class) in found {
            seen.entry(name)
                .or_default()
                .entry(class)
                .or_default()
                .push(what);
        }
    }
    // The fixture does what it is for: Today's strip and the dawn rings rank the birds in
    // different orders, so a colour by rank would differ between them.
    assert_ne!(
        first_named[0].1, first_named[1].1,
        "precondition: the strip and the rings lead with different birds: {first_named:?}"
    );

    let mut twice = Vec::new();
    for (name, classes) in &seen {
        if classes.len() > 1 {
            twice.push(format!("{name}: {classes:?}"));
        }
    }
    assert!(
        twice.is_empty(),
        "a species is drawn in more than one colour:\n{}",
        twice.join("\n")
    );

    // Counterpart: an owned colour belongs to one species.
    let mut owner: BTreeMap<&str, &str> = BTreeMap::new();
    for (name, classes) in &seen {
        for class in classes.keys().filter(|c| c.as_str() != "ck-cother") {
            if let Some(other) = owner.insert(class, name) {
                panic!("{other} and {name} share {class}");
            }
        }
    }
    // The owners are the history's top eight, in the history's order, whatever today did.
    for (k, (com, _, _)) in HISTORY.iter().take(8).enumerate() {
        if let Some(classes) = seen.get(*com) {
            assert!(
                classes.contains_key(&format!("ck-c{}", k + 1)),
                "{com} is the history's number {} and owns ck-c{}: {classes:?}",
                k + 1,
                k + 1
            );
        }
    }
    for neutral in [HISTORY[8].0, NEWCOMER.0] {
        if let Some(classes) = seen.get(neutral) {
            assert!(
                classes.keys().all(|c| c == "ck-cother"),
                "{neutral} owns no colour and is drawn in the neutral: {classes:?}"
            );
        }
    }
    assert!(
        seen.contains_key(HISTORY[8].0) || seen.contains_key(NEWCOMER.0),
        "precondition: a species without a colour of its own is drawn somewhere"
    );

    // Today's songs: the scripts number the same colours 1 to 8, and 0 for the neutral.
    let feed = songs(&get(&state, "/pages/today-songs").await);
    assert!(feed.len() >= 2, "{feed:?}");
    for (name, number) in &feed {
        let want = HISTORY
            .iter()
            .take(8)
            .position(|(c, _, _)| c == name)
            .map_or(0, |k| k as u64 + 1);
        assert_eq!(*number, want, "{name} in Today's songs: {feed:?}");
    }
}

// ── The sweep, kept: every way a colour class is made ────────────────────────────

/// Where a categorical colour is made from a position or written as a literal, and why that
/// site is not a species. A species takes its colour from `kit::SpeciesSlots` and nowhere
/// else; a new site that makes a colour any other way fails below until it is classified here.
const NOT_SPECIES: &[(&str, &str, &str)] = &[
    // (file, what is matched, why it is not a species)
    (
        "src/routes/pages/atoms.rs",
        "literal",
        "a sparkline's stroke when its caller names none; every species sparkline names its species' colour",
    ),
    (
        "src/routes/pages/viz/kit.rs",
        "format-class",
        "slot_class and SpeciesSlots::class build the class",
    ),
    (
        "src/routes/pages/viz/kit.rs",
        "format-token",
        "SpeciesSlots::colour builds the token",
    ),
    (
        "src/routes/pages/viz/bars.rs",
        "slot_class",
        "Fill::Slot and Daily::slot: one colour for every bar of a total",
    ),
    (
        "src/routes/pages/viz/bars.rs",
        "literal",
        "the dawn and dusk hours (ck-c1) and the temperature line (ck-c5)",
    ),
    (
        "src/routes/pages/viz/ridge.rs",
        "literal",
        "the seasons: spring ck-c1, autumn ck-c5",
    ),
    (
        "src/routes/pages/viz/trend.rs",
        "literal",
        "one series, a daily total or richness, in ck-c1",
    ),
    (
        "src/routes/pages/viz/timeline.rs",
        "literal",
        "the life list: one series, species so far, in ck-c1",
    ),
    (
        "static/skysong.js",
        "js-template",
        "slotClass(slot): the station slot the server sends",
    ),
    (
        "static/terrain.js",
        "js-template",
        "the palette read from the tokens, and speciesColour(t): the station slot the server sends",
    ),
];

fn served_rust(src: &str) -> String {
    // The test module at the foot of a file, and comment lines, are not served.
    let end = src.find("#[cfg(test)]\nmod tests").unwrap_or(src.len());
    src[..end]
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn walk(dir: &std::path::Path, ext: &str, recurse: bool, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = entry.path();
        if p.is_dir() {
            if recurse {
                walk(&p, ext, recurse, out);
            }
        } else if p.extension().is_some_and(|e| e == ext) {
            out.push(p);
        }
    }
}

/// The kinds of colour-making each file holds.
fn colour_sites(rel: &str, text: &str) -> BTreeSet<&'static str> {
    let mut kinds = BTreeSet::new();
    if std::path::Path::new(rel)
        .extension()
        .is_some_and(|e| e == "rs")
    {
        if text.matches("slot_class(").count() > text.matches("fn slot_class(").count() {
            kinds.insert("slot_class");
        }
        if text.contains("\"ck-c{") || text.contains("ck-c{}") || text.contains("series_color(") {
            kinds.insert("format-class");
        }
        if text.contains("--chart-{") {
            kinds.insert("format-token");
        }
        let literal = (1..=8).any(|k| {
            let class = format!("ck-c{k}");
            let token = format!("--chart-{k})");
            text.match_indices(&class)
                .any(|(i, _)| !text[i + class.len()..].starts_with(|c: char| c.is_ascii_digit()))
                || text.contains(&token)
        });
        if literal {
            kinds.insert("literal");
        }
    } else if text.contains("ck-c${") || text.contains("--chart-${") {
        kinds.insert("js-template");
    }
    kinds
}

/// Every place a colour class or token is made is a species site that goes through the
/// station's assignment, or a named site that is not a species. Seen failing on the tree
/// before this change, which made species colours from `slot_class(i)` and `series_color(i)`
/// in twelve files.
#[test]
fn every_colour_made_by_position_is_named_and_is_not_a_species() {
    // `BNB_SWEEP_ROOT` points the sweep at another copy of the crate (an older tree, to watch
    // it go red); by default it reads this one.
    let root_buf = std::env::var_os("BNB_SWEEP_ROOT").map_or_else(
        || std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        std::path::PathBuf::from,
    );
    let root = root_buf.as_path();
    let mut files = Vec::new();
    walk(&root.join("src"), "rs", true, &mut files);
    walk(&root.join("static"), "js", false, &mut files);
    let mut found: BTreeSet<(String, &'static str)> = BTreeSet::new();
    for p in &files {
        let rel = p
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let src = std::fs::read_to_string(p).unwrap();
        let text = if p.extension().is_some_and(|e| e == "rs") {
            served_rust(&src)
        } else {
            src
        };
        for kind in colour_sites(&rel, &text) {
            found.insert((rel.clone(), kind));
        }
    }
    let named: BTreeSet<(String, &str)> = NOT_SPECIES
        .iter()
        .map(|(f, k, _)| ((*f).to_owned(), *k))
        .collect();
    let unnamed: Vec<_> = found
        .iter()
        .filter(|(f, k)| !named.contains(&(f.clone(), *k)))
        .collect();
    assert!(
        unnamed.is_empty(),
        "a colour is made by position or written as a literal here, and the site is not \
         classified: a species takes kit::SpeciesSlots; anything else is named in NOT_SPECIES \
         with why: {unnamed:?}"
    );
    // Counterpart: every named site still exists, so the list cannot outlive what it excuses.
    let stale: Vec<_> = named
        .iter()
        .filter(|(f, k)| !found.iter().any(|(ff, kk)| ff == f && kk == k))
        .collect();
    assert!(
        stale.is_empty(),
        "named sites that no longer exist: {stale:?}"
    );
}
