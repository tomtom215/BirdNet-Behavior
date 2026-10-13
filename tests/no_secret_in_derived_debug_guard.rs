//! No type that holds a credential may derive `Debug`.
//!
//! # Why
//!
//! A derived `Debug` prints every field in full. A config or client struct
//! gets formatted with `{:?}` sooner or later — a `tracing::debug!` added
//! during an unrelated investigation, an `expect` message, a panic in a test
//! that CI then publishes — and the password, bot token or camera URL in it
//! goes wherever that line goes. The support bundle is one such place, and
//! operators are asked to attach it to public bug reports.
//!
//! The project already hand-writes `Debug` for exactly this reason in a few
//! places (`FlickrClient`, `offsite::sigv4::Credentials`, `ApiToken`). A sweep
//! found 26 more that derived it: SMTP and MQTT passwords, Discord and
//! Telegram tokens, ntfy and Slack credentials, RTSP URLs with a password in
//! them, webhook auth values, the login and onboarding forms, and the command
//! line's own flags. A 27th, `dispatch::Body`, was invisible to this guard —
//! Pushover's token is a form *value*, not a field name — and was found by
//! `birdnet-integrations`' `debug_never_prints_a_credential`, which formats
//! each of these types with a secret in it.
//!
//! # What the guard looks at
//!
//! Every `#[derive(..Debug..)]` struct or enum in production source, and each
//! named field in it whose name has a credential-shaped segment (`password`,
//! `token`, `api_key`, `url`, …) and whose type is not a number or a flag. A
//! name is a heuristic, so what it flags that is *not* a credential is listed
//! below by name, each with the reason it is safe — a list someone has to
//! argue with is harder to grow carelessly than a looser pattern.
//!
//! It does not see a credential under a name it does not recognise, a tuple
//! variant's unnamed field (`NtfyAuth::Token(String)`), or a field written on
//! the same line as its struct — rustfmt never writes one. A field whose type
//! has its own hand-written `Debug` is trusted to that impl; a field of a type
//! that *derives* `Debug` is judged where that type is defined.

use std::path::{Path, PathBuf};

/// Flagged types that hold no credential: `(path suffix, type, why)`.
const NOT_SECRET: &[(&str, &str, &str)] = &[
    (
        "birdnet-db/src/sqlite/queries/images.rs",
        "ImageBlacklist",
        "a public image URL an admin chose to hide",
    ),
    (
        "birdnet-web/src/routes/admin/images.rs",
        "BlacklistForm",
        "the same public image URL, as submitted",
    ),
    (
        "birdnet-integrations/src/auto_update/mod.rs",
        "UpdateInfo",
        "a public GitHub release asset URL",
    ),
    (
        "birdnet-integrations/src/dispatch/plan.rs",
        "Message",
        "the station's own image link for a detection",
    ),
    (
        "birdnet-integrations/src/email/types.rs",
        "DetectionEmail",
        "the station's own link to a detection",
    ),
    (
        "birdnet-integrations/src/notification.rs",
        "NotificationContext",
        "the station's own listen, image and station links",
    ),
    (
        "birdnet-integrations/src/model_catalog.rs",
        "CatalogEntry",
        "compiled-in public download URLs",
    ),
    (
        "birdnet-integrations/src/species_images/types.rs",
        "SpeciesImage",
        "public Wikipedia / Flickr image and page URLs",
    ),
    (
        "birdnet-integrations/src/weather.rs",
        "Client",
        "`base_url` is the provider's API origin; the key is in `Provider`, whose Debug is hand-written",
    ),
    (
        "birdnet-web/src/routes/admin/rules.rs",
        "ImportOutcome",
        "`needs_credential` lists the names of rules imported without one",
    ),
    (
        "birdnet-integrations/src/offsite/s3.rs",
        "Addressed",
        "an object URL; SigV4 signs in a header, not the query",
    ),
];

/// Field-name segments that name a credential, or a URL that may carry one.
const SECRET_SEGMENTS: &[&str] = &[
    "password",
    "passwd",
    "passphrase",
    "secret",
    "token",
    "auth",
    "authorization",
    "credential",
    "credentials",
    "url",
    "urls",
];

/// Types that cannot carry a credential however the field is named.
const SAFE_TYPES: &[&str] = &[
    "bool", "f32", "f64", "u8", "u16", "u32", "u64", "usize", "i32", "i64",
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if !matches!(name, "target" | "tests" | "examples" | "benches") {
                collect(&path, out);
            }
        } else if path.extension().and_then(|x| x.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Index just past the `close` that balances the `open` at `start`.
fn balanced(s: &[u8], start: usize, open: u8, close: u8) -> usize {
    let mut depth = 0usize;
    for (i, &c) in s.iter().enumerate().skip(start) {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return i + 1;
            }
        }
    }
    s.len()
}

/// Source with every `#[cfg(test)] mod … { … }` blanked out.
fn without_test_modules(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while let Some(off) = src[i..].find("#[cfg(test)]") {
        let at = i + off;
        let rest = src[at + "#[cfg(test)]".len()..].trim_start();
        if rest.starts_with("mod ")
            && let Some(b) = src[at..].find('{')
        {
            out.push_str(&src[i..at]);
            i = balanced(bytes, at + b, b'{', b'}');
            continue;
        }
        out.push_str(&src[i..=at]);
        i = at + 1;
    }
    out.push_str(&src[i..]);
    out
}

/// A field line's name and type, if the line declares a named field.
fn field(line: &str) -> Option<(&str, &str)> {
    let mut l = line.trim();
    if l.starts_with("//") || l.starts_with('#') {
        return None;
    }
    if let Some(rest) = l.strip_prefix("pub") {
        l = rest.trim_start();
        if l.starts_with('(') {
            l = l[l.find(')')? + 1..].trim_start();
        }
    }
    let (name, ty) = l.split_once(':')?;
    let name = name.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    if ty.starts_with(':') {
        return None; // a path, `a::b`, not a field
    }
    Some((name, ty.trim().trim_end_matches(',').trim()))
}

fn is_secret_name(name: &str) -> bool {
    let segs: Vec<&str> = name.split('_').collect();
    segs.iter().any(|s| SECRET_SEGMENTS.contains(s))
        || segs.windows(2).any(|w| w == ["api", "key"])
        || name == "apikey"
}

/// Names of types with a hand-written `impl … Debug for Name`. A field of one
/// of these types prints whatever that impl decides, which is the point.
fn hand_written_debug(files: &[PathBuf]) -> Vec<String> {
    let mut out = Vec::new();
    for path in files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in src.lines() {
            let l = line.trim();
            if !l.starts_with("impl") || !l.contains("Debug for ") {
                continue;
            }
            let after = &l[l.find("Debug for ").unwrap_or(0) + "Debug for ".len()..];
            let name: String = after
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                out.push(name);
            }
        }
    }
    out
}

/// `Option<Passphrase>` → `Passphrase`; `crate::x::Token` → `Token`.
fn innermost(ty: &str) -> &str {
    let mut t = ty.trim();
    while let Some(inner) = t.strip_prefix("Option<").and_then(|r| r.strip_suffix('>')) {
        t = inner.trim();
    }
    t.rsplit("::").next().unwrap_or(t)
}

/// `(rel path, type name, offending field lines)` for every flagged item.
fn scan(root: &Path) -> (usize, Vec<(String, String, Vec<String>)>) {
    let mut files = Vec::new();
    collect(&root.join("src"), &mut files);
    collect(&root.join("crates"), &mut files);
    files.sort();
    let hand_written = hand_written_debug(&files);
    let mut hits = Vec::new();
    for path in &files {
        let Ok(raw) = std::fs::read_to_string(path) else {
            continue;
        };
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string();
        let src = without_test_modules(&raw);
        let b = src.as_bytes();
        let mut i = 0;
        while let Some(off) = src[i..].find("#[derive(") {
            let at = i + off;
            let end = balanced(b, at + 1, b'[', b']');
            i = end;
            if !src[at..end]
                .split(|c: char| !c.is_alphanumeric())
                .any(|w| w == "Debug")
            {
                continue;
            }
            // Skip further attributes (`#[serde(..)]`, doc comments) to the item.
            let mut j = end;
            loop {
                let rest = &src[j..];
                let t = rest.trim_start();
                j += rest.len() - t.len();
                if t.starts_with("#[") {
                    j = balanced(b, j + 1, b'[', b']');
                } else if t.starts_with("///") || t.starts_with("//") {
                    j += t.find('\n').unwrap_or(t.len());
                } else {
                    break;
                }
            }
            let head = &src[j..];
            let head = head
                .strip_prefix("pub(crate) ")
                .or_else(|| head.strip_prefix("pub(super) "))
                .or_else(|| head.strip_prefix("pub "))
                .unwrap_or(head);
            let Some(rest) = head
                .strip_prefix("struct ")
                .or_else(|| head.strip_prefix("enum "))
            else {
                continue;
            };
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            let (Some(brace), semi) = (head.find('{'), head.find(';')) else {
                continue;
            };
            if semi.is_some_and(|s| s < brace) {
                continue; // tuple or unit struct
            }
            let start = j + (src[j..].len() - head.len()) + brace;
            let body = &src[start..balanced(b, start, b'{', b'}')];
            let bad: Vec<String> = body
                .lines()
                .filter_map(field)
                .filter(|(n, ty)| {
                    is_secret_name(n)
                        && !SAFE_TYPES.contains(ty)
                        && !SAFE_TYPES.iter().any(|t| *ty == format!("Option<{t}>"))
                        && !hand_written.contains(&innermost(ty).to_owned())
                })
                .map(|(n, ty)| format!("{n}: {ty}"))
                .collect();
            if !bad.is_empty() {
                hits.push((rel.clone(), name, bad));
            }
        }
    }
    (files.len(), hits)
}

fn exempt(rel: &str, ty: &str) -> bool {
    NOT_SECRET
        .iter()
        .any(|(p, t, _)| rel.ends_with(p) && *t == ty)
}

#[test]
fn no_type_holding_a_credential_derives_debug() {
    let (_, hits) = scan(&workspace_root());
    let failures: Vec<String> = hits
        .iter()
        .filter(|(rel, ty, _)| !exempt(rel, ty))
        .map(|(rel, ty, fields)| format!("  {rel}: {ty} {{ {} }}", fields.join(", ")))
        .collect();
    assert!(
        failures.is_empty(),
        "{} type(s) derive Debug over a credential-shaped field. Write Debug by \
         hand and print the secret as \"<redacted>\" (see FlickrClient), or, if \
         the field holds no credential, add the type to NOT_SECRET with the \
         reason:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The counterpart: the scanner finds what it is meant to find. Without this,
/// a parser that never matched a derive would pass the guard above forever.
#[test]
fn the_scanner_flags_a_derived_debug_over_a_password() {
    let src = "#[derive(Debug, Clone)]\n/// doc\n#[serde(default)]\npub struct Smtp {\n    pub host: String,\n    pub password: String,\n    pub port: u16,\n    pub auth_tokens: u64,\n}\n\n#[derive(Clone)]\nstruct Hand {\n    password: String,\n}\nimpl std::fmt::Debug for Hand {\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n        f.write_str(\"Hand\")\n    }\n}\n#[derive(Debug)]\nstruct Holder {\n    password: Option<Hand>,\n    token: String,\n}\n\n#[cfg(test)]\nmod tests {\n    #[derive(Debug)]\n    struct Fixture {\n        password: String,\n    }\n}\n";
    let dir = tempfile_dir();
    std::fs::create_dir_all(dir.join("src")).expect("mkdir");
    std::fs::write(dir.join("src/lib.rs"), src).expect("write");
    let (_, hits) = scan(&dir);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(
        hits,
        vec![
            (
                "src/lib.rs".to_owned(),
                "Smtp".to_owned(),
                vec!["password: String".to_owned()]
            ),
            (
                "src/lib.rs".to_owned(),
                "Holder".to_owned(),
                vec!["token: String".to_owned()]
            ),
        ],
        "only the derived secrets: not the port, not a numeric `tokens`, not a \
         hand-written type, not a field whose type writes its own Debug, not a \
         test fixture"
    );
}

/// Every exemption still names a type the scanner flags; a stale one would
/// silently allow whatever later took that name.
#[test]
fn the_exemptions_are_still_needed() {
    let (scanned, hits) = scan(&workspace_root());
    assert!(scanned >= 100, "scanned only {scanned} files");
    let stale: Vec<_> = NOT_SECRET
        .iter()
        .filter(|(p, t, _)| !hits.iter().any(|(rel, ty, _)| rel.ends_with(p) && ty == t))
        .map(|(p, t, _)| format!("{p}: {t}"))
        .collect();
    assert!(
        stale.is_empty(),
        "exemptions with nothing to exempt: {stale:?}"
    );
}

fn tempfile_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bnb-debug-guard-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}
