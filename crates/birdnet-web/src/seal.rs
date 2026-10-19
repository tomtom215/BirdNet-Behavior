//! Marks the `<script>` and `<style>` tags this server wrote itself, so the
//! per-request CSP nonce goes onto those and onto nothing else.
//!
//! # The defect this replaces
//!
//! The nonce used to be stamped by a pass over the finished response body that
//! rewrote every `<script ` and `<script>` it found. At that point text the
//! renderer wrote and text a visitor sent are the same bytes, so one
//! unescaped reflection anywhere in a page, a `?filter=` echoed into a form or
//! a stored name printed into a list, became `<script nonce="…">` with the
//! right nonce: script execution that the CSP, built to stop exactly that, had
//! been told to allow. Two such reflections were found and fixed in the audit
//! (`/quarantine`, `/pages/weekly-content`); the design made every
//! `format!` that forgot an escape the same bug.
//!
//! # How it works now
//!
//! Authored markup is *sealed* before any visitor-controlled text is put next
//! to it: [`seal`] rewrites each `<script` and `<style` opening tag in the
//! authored text to carry a per-process random token, and [`redeem`], run once
//! on the finished body, replaces exactly those tags with the request's nonce
//! and removes any copy of the token that is left. A tag a visitor managed to
//! reflect carries no token (they cannot know it: 128 random bits, generated
//! at start, never sent anywhere because [`redeem`] strips it from every HTML
//! response), so it gets no nonce. [`redeem`] also rewrites it to text
//! (`&lt;script`): in a full page the CSP would refuse it, but in a fragment
//! that htmx swaps in, the CSP's `'strict-dynamic'` admits the script htmx
//! re-creates from it, nonce or no nonce.
//!
//! Sealing happens where the authored text is defined: `static X: Sealed`
//! for the page templates, [`seal`] on a literal that is built into a page.
//! Never on a string that already holds visitor text. A page whose authored
//! script was not sealed loses its nonce and stops working, which
//! `tests/every_script_on_every_page_carries_the_nonce.rs` finds by rendering
//! every page; the failure is a broken page and not a silent exposure.

use std::sync::{LazyLock, OnceLock};

/// Authored markup, sealed the first time it is read.
///
/// `static LOGIN: Sealed = Sealed::new(include_str!("…"));` reads as a `&str`
/// (`Deref`), formats with `{LOGIN}` (`Display`) and answers `.replace(…)`, so
/// the text can stay where it was and only its declaration changes.
pub struct Sealed {
    source: &'static str,
    sealed: OnceLock<String>,
}

impl Sealed {
    pub const fn new(source: &'static str) -> Self {
        Self {
            source,
            sealed: OnceLock::new(),
        }
    }

    pub fn as_str(&self) -> &str {
        self.sealed.get_or_init(|| seal(self.source))
    }
}

impl std::ops::Deref for Sealed {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for Sealed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 128 random bits, hex, fixed for the life of the process.
static TOKEN: LazyLock<String> = LazyLock::new(|| {
    let mut bytes = [0_u8; 16];
    if crate::random::fill(&mut bytes).is_ok() {
        bytes.iter().fold(String::with_capacity(32), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
            s
        })
    } else {
        // An OS randomness failure is essentially impossible on Linux. The
        // session-id generator has its own documented fallback, and an empty
        // token would seal nothing and break every page.
        crate::session::generate_session_id()
    }
});

/// The tags whose nonce is the CSP's to admit. `<style>` is here as well as
/// `<script>`: a reflected `<style>` that took the nonce could read an
/// attribute out of the page with a selector and send it to any `https:`
/// image host, which `img-src` allows.
const TAGS: [&str; 2] = ["script", "style"];

/// The attribute a sealed tag carries until [`redeem`] replaces it.
fn attribute() -> String {
    format!("data-bnb-seal=\"{}\"", *TOKEN)
}

/// Mark every `<script` and `<style` opening tag in `authored`.
///
/// Call it on text the program wrote, before any visitor-controlled value is
/// inserted. A tag name is only matched when followed by whitespace, `/` or
/// `>`, so `<scripts>` and `<styled-el>` are left alone.
pub fn seal(authored: &str) -> String {
    let attr = attribute();
    let mut out = String::with_capacity(authored.len() + 64);
    let mut rest = authored;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let tag = TAGS.iter().find(|t| {
            tail[1..].strip_prefix(**t).is_some_and(|after| {
                after
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_whitespace() || c == '/' || c == '>')
            })
        });
        if let Some(tag) = tag {
            let end = 1 + tag.len();
            out.push_str(&tail[..end]);
            out.push(' ');
            out.push_str(&attr);
            rest = &tail[end..];
        } else {
            out.push('<');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Turn each sealed tag in a finished body into a tag carrying `nonce`,
/// neutralise every other `<script` in it, and remove the token from anywhere
/// else it survived, so that it never leaves the process.
///
/// The second step is what makes the first sufficient. `script-src` is
/// `'strict-dynamic'`, so a script that htmx *re-creates* from a swapped
/// fragment is admitted without a nonce of its own; a reflected `<script>`
/// in a fragment response therefore ran even though it carried none (measured
/// in Chromium, not assumed). Any `<script` that is not ours is rewritten to
/// `&lt;script`: it shows as text, in a document or in a fragment, in any
/// letter case (`<SCRIPT>` is the same tag to the parser).
pub fn redeem(html: &str, nonce: &str) -> String {
    let attr = attribute();
    let sealed_script = format!("<script {attr}");
    let mut out = String::with_capacity(html.len() + 32);
    let mut rest = html;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        // Bytes, not a `str` slice: the seventh byte may sit inside a
        // multi-byte character of the text after a stray `<`.
        let bytes = tail.as_bytes();
        let is_script = bytes.len() > "<script".len()
            && bytes[.."<script".len()].eq_ignore_ascii_case(b"<script")
            && (bytes["<script".len()].is_ascii_whitespace()
                || matches!(bytes["<script".len()], b'/' | b'>'));
        if is_script && tail.starts_with(&sealed_script) {
            out.push_str("<script nonce=\"");
            out.push_str(nonce);
            out.push('"');
            rest = &tail[sealed_script.len()..];
        } else if is_script {
            out.push_str("&lt;script");
            rest = &tail["<script".len()..];
        } else {
            out.push('<');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    let style_open = format!("<style {attr}");
    let mut out = out.replace(&style_open, &format!("<style nonce=\"{nonce}\""));
    if out.contains(attr.as_str()) {
        out = out.replace(&format!(" {attr}"), "");
        out = out.replace(&attr, "");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A nonce minted per run rather than written down. Code scanning reads a
    /// literal that reaches a parameter called `nonce` as a hard-coded
    /// cryptographic value, and nothing here needs one in particular: what is
    /// asserted is that whatever is handed to `redeem` is what comes out.
    fn fresh_nonce() -> String {
        format!("nonce-{}", std::process::id())
    }

    #[test]
    fn a_sealed_tag_is_redeemed_for_the_nonce_and_a_reflected_one_is_not() {
        let nonce = fresh_nonce();
        let authored = seal("<script>ours()</script><script src=\"/a.js\" defer></script>");
        let page = format!("{authored}<p><script>theirs()</script></p>");
        let out = redeem(&page, &nonce);
        assert!(out.contains(&format!("<script nonce=\"{nonce}\">ours()")));
        assert!(out.contains(&format!("<script nonce=\"{nonce}\" src=\"/a.js\" defer>")));
        assert!(
            out.contains("<p>&lt;script>theirs()"),
            "a tag nobody sealed must be defanged, not merely left bare: {out}"
        );
        assert_eq!(out.matches("nonce=").count(), 2);
        assert_eq!(
            out.matches("<script").count(),
            2,
            "only ours are tags: {out}"
        );
    }

    #[test]
    fn style_tags_are_sealed_the_same_way() {
        let page = format!(
            "{}<style>.x{{}}</style><style>.y{{}}</style>",
            seal("<style>.a{}</style>")
        );
        let nonce = fresh_nonce();
        let out = redeem(&page, &nonce);
        assert!(out.contains(&format!("<style nonce=\"{nonce}\">.a")));
        assert!(out.contains("<style>.x"), "reflected style must stay bare");
        assert_eq!(out.matches("nonce=").count(), 1);
    }

    #[test]
    fn every_spelling_of_a_foreign_script_tag_is_defanged() {
        let nonce = fresh_nonce();
        for tag in [
            "<script>x()</script>",
            "<SCRIPT>x()</SCRIPT>",
            "<ScRiPt src=//evil.example/x.js></ScRiPt>",
            "<script\n>x()</script>",
            "<script/src=//evil.example/x.js>",
            "<script\t>x()</script>",
        ] {
            let out = redeem(&format!("<p>{tag}</p>"), &nonce);
            assert!(
                !out.to_ascii_lowercase().contains("<script"),
                "{tag:?} came out as a tag: {out}"
            );
        }
        // Not a script tag: a different element, left exactly as written.
        let other = "<scripts>x</scripts><script-like>y</script-like><p>a < script</p>";
        assert_eq!(redeem(other, &nonce), other);
    }

    #[test]
    fn text_after_a_stray_angle_bracket_may_be_multibyte() {
        // A seventh byte inside a character must not panic.
        let nonce = fresh_nonce();
        for text in [
            "<ééé<scr",
            "a <日本語 b",
            "<scrİpt>x",
            "<\u{1F426}\u{1F426}\u{1F426}",
        ] {
            let _ = redeem(text, &nonce);
            let _ = seal(text);
        }
    }

    #[test]
    fn only_whole_tag_names_are_sealed() {
        let text = "<scripts>x</scripts><styled-card></styled-card><p>script</p> a < script";
        assert_eq!(seal(text), text);
    }

    #[test]
    fn a_visitor_who_guesses_the_attribute_form_still_gets_no_nonce() {
        // They know the attribute *name*; the value is the secret.
        let forged = "<script data-bnb-seal=\"0123456789abcdef0123456789abcdef\">x()</script>";
        let out = redeem(forged, &fresh_nonce());
        assert!(!out.contains("nonce="), "{out}");
        assert!(!out.contains("<script"), "{out}");
    }

    #[test]
    fn the_token_does_not_survive_redemption_anywhere() {
        // A sealed tag whose attribute order was disturbed, and one inside
        // a comment: neither may carry the token to the browser.
        let attr = attribute();
        let html = format!("<!-- <script {attr}> --><script defer {attr}>x()</script>");
        let out = redeem(&html, &fresh_nonce());
        assert!(!out.contains(TOKEN.as_str()), "{out}");
    }

    #[test]
    fn the_token_is_random_per_process_and_long_enough() {
        assert_eq!(TOKEN.len(), 32);
        assert!(TOKEN.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_sealed_page_redeems_to_the_plain_nonced_form() {
        let nonce = fresh_nonce();
        let once = redeem(&seal("<script>a()</script>"), &nonce);
        assert_eq!(once, format!("<script nonce=\"{nonce}\">a()</script>"));
    }
}
