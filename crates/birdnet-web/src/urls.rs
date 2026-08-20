//! Percent-encoding, in one place, with the distinction between the two kinds
//! spelled out.
//!
//! There were three encoders in this crate. Two — `auth_middleware` and
//! `routes::auth_pages` — were byte-identical, both building the `?next=`
//! target for a login redirect. The third looked like a fourth copy and is not:
//! it escapes `/` and the others deliberately do not, because they encode a
//! whole path and it encodes a single segment or query value.
//!
//! That is exactly the shape of duplication worth collapsing: the copies that
//! are the same become one, and the one that differs gets a name and a reason
//! instead of looking like a transcription error waiting to be "fixed".

use std::fmt::Write as _;

/// Percent-encode a URL **path**, preserving `/`.
///
/// For building a redirect target such as `?next=/admin/settings`: the slashes
/// are structure, not data, and encoding them would send the operator to a
/// path that does not exist.
///
/// Everything outside RFC 3986's unreserved set (plus `/`) is escaped, which
/// includes the `?`, `#` and `&` that would otherwise let a crafted path break
/// out of the query parameter it is being embedded in.
#[must_use]
pub fn encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

/// Percent-encode a single path **segment** or query value, escaping `/`.
///
/// For a species name in `/species/Great%20Tit` or a value in a query string.
/// A name containing a slash must not become two path segments — which is the
/// whole reason this differs from [`encode_path`], and why the two are not
/// interchangeable.
#[must_use]
pub fn encode_segment(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::{encode_path, encode_segment};

    /// The one difference between them, asserted directly. Anything that reads
    /// like a copy-paste slip here is a real behaviour change.
    #[test]
    fn only_the_slash_distinguishes_them() {
        assert_eq!(encode_path("/admin/settings"), "/admin/settings");
        assert_eq!(encode_segment("/admin/settings"), "%2Fadmin%2Fsettings");
        // Everything else must agree.
        for s in ["Great Tit", "Erithacus rubecula", "a?b#c&d", "", "~-_."] {
            assert_eq!(
                encode_path(s),
                encode_segment(s),
                "only `/` may differ, but {s:?} did"
            );
        }
    }

    /// A path is embedded in `?next=`, so the characters that would break out
    /// of a query parameter have to be escaped.
    #[test]
    fn a_path_cannot_break_out_of_the_query_parameter_it_lands_in() {
        assert_eq!(encode_path("/a?x=1"), "/a%3Fx%3D1");
        assert_eq!(encode_path("/a#frag"), "/a%23frag");
        assert_eq!(encode_path("/a&b=c"), "/a%26b%3Dc");
        assert_eq!(encode_path("/a b"), "/a%20b");
    }

    /// Multi-byte input is encoded per byte, not per char — a `%` followed by
    /// half a code point would be a malformed URL.
    #[test]
    fn non_ascii_is_encoded_byte_by_byte() {
        assert_eq!(encode_segment("é"), "%C3%A9");
        assert_eq!(encode_path("/Grünspecht"), "/Gr%C3%BCnspecht");
    }

    /// The unreserved set is left alone; encoding it would still be *correct*
    /// but would make every URL in the app unreadable.
    #[test]
    fn the_unreserved_set_passes_through() {
        let unreserved = "AZaz09-_.~";
        assert_eq!(encode_path(unreserved), unreserved);
        assert_eq!(encode_segment(unreserved), unreserved);
    }
}
