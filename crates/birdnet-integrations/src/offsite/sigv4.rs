//! AWS Signature Version 4 request signing, for the S3-compatible backup target.
//!
//! # Why this is written out rather than pulled in
//!
//! The alternative is the AWS SDK, which for one `PUT`, one `GET ?list-type=2`
//! and one `DELETE` brings a dependency tree larger than the rest of this
//! binary put together, onto a board where the release build is already
//! dominated by ONNX Runtime and DuckDB. The signing algorithm itself is four
//! HMAC-SHA256 calls over strings that are entirely specified, and both
//! primitives were already in the tree for the share-link tokens.
//!
//! What that trades away is the SDK's accumulated knowledge of every endpoint
//! quirk. So this signs exactly the three requests the backup target makes, and
//! is checked against a reference implementation rather than against a reading
//! of the specification — see `tests/sigv4_vectors.rs`, whose vectors come from
//! botocore, and one of which is AWS's own published example.
//!
//! # The parts that are easy to get wrong
//!
//! **The path is encoded once, not twice.** S3 is the exception among AWS
//! services: its canonical URI is the percent-encoded path as it appears on the
//! wire, where other services encode it a second time. Callers therefore hand
//! [`sign`] a *raw* path, and [`encode_path`] produces both the canonical form
//! and the request URL, so the two cannot disagree — which is the shape of the
//! bug that produces `SignatureDoesNotMatch` on exactly the keys that contain a
//! space.
//!
//! **The query string is sorted after encoding, not before.** Sorting raw keys
//! and then encoding gives a different order whenever an encoded character
//! changes the comparison, and the failure is intermittent by key name.
//!
//! **`host` carries the port** when there is one. A self-hosted `MinIO` on :9000
//! signs `host:minio.example.net:9000`; dropping the port is the standard
//! way a first S3-compatible integration fails against everything except AWS.

use std::fmt::Write as _;

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

/// The algorithm name that appears in the credential scope and the header.
const ALGORITHM: &str = "AWS4-HMAC-SHA256";

/// The service name in the credential scope. Only S3 is signed here.
const SERVICE: &str = "s3";

/// SHA-256 of the empty string, the payload hash for every request without a body.
pub const EMPTY_PAYLOAD_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Long-lived S3 credentials.
///
/// No session-token support: a station holds a key an operator created for it
/// and keeps it for years. STS credentials expire in hours and would need a
/// refresh path that nothing here could exercise.
#[derive(Clone)]
pub struct Credentials {
    /// The access key ID (`AKIA…`), which appears in the Authorization header.
    pub access_key: String,
    /// The secret key. Never logged, never echoed, and deliberately not
    /// `Debug`-printable — see the manual `Debug` below.
    pub secret_key: String,
}

impl std::fmt::Debug for Credentials {
    /// Redacts the secret.
    ///
    /// Derived `Debug` would put the secret key into any `tracing` line that
    /// formatted a config struct, and into the support bundle, which is a file
    /// operators are asked to attach to public bug reports.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("access_key", &self.access_key)
            .field("secret_key", &"<redacted>")
            .finish()
    }
}

/// Everything about a request that the signature covers.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    /// Uppercase HTTP method.
    pub method: &'a str,
    /// Host, including the port when the URL carries one.
    pub host: &'a str,
    /// Path with a leading `/`, **not** percent-encoded.
    pub path: &'a str,
    /// Query parameters, keys and values **not** percent-encoded. Order here is
    /// irrelevant; the canonical form is sorted.
    pub query: &'a [(String, String)],
    /// Hex SHA-256 of the request body, or [`EMPTY_PAYLOAD_SHA256`].
    pub payload_sha256: &'a str,
    /// Additional headers to sign, names already lowercase.
    pub extra_headers: &'a [(String, String)],
    /// Region for the credential scope (`us-east-1` and friends).
    pub region: &'a str,
    /// `YYYYMMDDTHHMMSSZ`, the value of `x-amz-date`.
    pub timestamp: &'a str,
}

/// The signed request: the header value, and the headers it covers.
#[derive(Debug, Clone)]
pub struct Signed {
    /// Value for the `Authorization` header.
    pub authorization: String,
    /// Every header that must be sent, in `(name, value)` form. Sending fewer
    /// or more than these invalidates the signature, so the caller sets exactly
    /// this list rather than assembling its own.
    pub headers: Vec<(String, String)>,
    /// The canonical request, kept for diagnostics.
    ///
    /// An S3-compatible service that rejects a signature returns its own
    /// canonical request in the error body; having ours to print beside it
    /// turns "`SignatureDoesNotMatch`" from a guess into a diff.
    pub canonical_request: String,
}

/// Sign a request.
///
/// Returns the `Authorization` value along with the exact header set it covers.
#[must_use]
pub fn sign(creds: &Credentials, req: &Request<'_>) -> Signed {
    // Headers to sign: the three that are always present, plus the caller's.
    let mut headers: Vec<(String, String)> = vec![
        ("host".to_owned(), req.host.to_owned()),
        (
            "x-amz-content-sha256".to_owned(),
            req.payload_sha256.to_owned(),
        ),
        ("x-amz-date".to_owned(), req.timestamp.to_owned()),
    ];
    for (name, value) in req.extra_headers {
        headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
    }
    headers.sort_by(|a, b| a.0.cmp(&b.0));

    let canonical_headers: String = headers
        .iter()
        .map(|(n, v)| format!("{n}:{v}\n"))
        .collect::<Vec<_>>()
        .concat();
    let signed_headers: String = headers
        .iter()
        .map(|(n, _)| n.as_str())
        .collect::<Vec<_>>()
        .join(";");

    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        req.method,
        encode_path(req.path),
        canonical_query(req.query),
        canonical_headers,
        signed_headers,
        req.payload_sha256,
    );

    let date = &req.timestamp[..8.min(req.timestamp.len())];
    let scope = format!("{date}/{}/{SERVICE}/aws4_request", req.region);
    let string_to_sign = format!(
        "{ALGORITHM}\n{}\n{scope}\n{}",
        req.timestamp,
        hex(&sha256(canonical_request.as_bytes())),
    );

    let signing_key = signing_key(&creds.secret_key, date, req.region);
    let signature = hex(&hmac(&signing_key, string_to_sign.as_bytes()));

    let authorization = format!(
        "{ALGORITHM} Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        creds.access_key,
    );

    Signed {
        authorization,
        headers,
        canonical_request,
    }
}

/// Percent-encode a path for both the canonical request and the request URL.
///
/// `/` is left alone because it separates key segments; everything outside the
/// RFC 3986 unreserved set is encoded, uppercase-hex, which is what `SigV4`
/// specifies and what a mismatched encoder gets wrong for `+`, space and `=`.
#[must_use]
pub fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        if is_unreserved(byte) || byte == b'/' {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// Percent-encode a query key or value: everything but the unreserved set,
/// `/` included.
#[must_use]
pub fn encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if is_unreserved(byte) {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// The canonical query string: encode first, then sort, then join.
#[must_use]
pub fn canonical_query(params: &[(String, String)]) -> String {
    let mut encoded: Vec<(String, String)> = params
        .iter()
        .map(|(k, v)| (encode_component(k), encode_component(v)))
        .collect();
    encoded.sort();
    encoded
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// RFC 3986 unreserved: `A-Z a-z 0-9 - _ . ~`.
const fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~')
}

/// The four-step HMAC chain that turns a secret into a date/region/service key.
fn signing_key(secret: &str, date: &str, region: &str) -> Vec<u8> {
    let k_date = hmac(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k_region = hmac(&k_date, region.as_bytes());
    let k_service = hmac(&k_region, SERVICE.as_bytes());
    hmac(&k_service, b"aws4_request")
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key).expect("HMAC-SHA256 accepts a key of any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// SHA-256 of a byte slice.
#[must_use]
pub fn sha256(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

/// Lowercase hex, which is what `SigV4` wants everywhere a digest appears.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_empty_payload_constant_is_the_hash_it_claims_to_be() {
        // A typo here fails every request with no body, which is two of the
        // three the backup target makes.
        assert_eq!(hex(&sha256(b"")), EMPTY_PAYLOAD_SHA256);
    }

    #[test]
    fn path_encoding_keeps_separators_and_encodes_the_rest() {
        assert_eq!(encode_path("/a/b.txt"), "/a/b.txt");
        assert_eq!(encode_path("/a b/c+d~e"), "/a%20b/c%2Bd~e");
        // Uppercase hex: SigV4 specifies it, and a lowercase encoder produces a
        // signature that differs only for keys with unusual characters.
        assert_eq!(encode_path("/\u{e9}"), "/%C3%A9");
    }

    #[test]
    fn query_components_encode_the_slash_that_paths_keep() {
        // The one difference between the two encoders, and the reason there are
        // two: a `prefix=stations/pi-1/` parameter must become `%2F`, while the
        // same characters in the path must not.
        assert_eq!(encode_component("stations/pi-1/"), "stations%2Fpi-1%2F");
        assert_eq!(encode_path("/stations/pi-1/"), "/stations/pi-1/");
    }

    #[test]
    fn the_query_string_is_sorted_after_encoding_not_before() {
        // The two orders differ only when an unreserved character is compared
        // against one that gets encoded, and the encoded form wins because `%`
        // is 0x25 — below every character in the unreserved set.
        //
        // The first version of this test used `b+c` against `b-c` and proved
        // nothing: `+` (0x2B) sorts before `-` (0x2D) raw, and `%2B` sorts
        // before `-` too, so both orders agree. `-` against `:` is the case
        // where they disagree: raw, `a-b` < `a:b` (0x2D < 0x3A); encoded,
        // `a%3Ab` < `a-b` (0x25 < 0x2D).
        let params = vec![
            ("a-b".to_owned(), "1".to_owned()),
            ("a:b".to_owned(), "2".to_owned()),
        ];
        assert_eq!(
            canonical_query(&params),
            "a%3Ab=2&a-b=1",
            "the parameters were sorted before they were encoded"
        );

        // Sorting is not the only thing that could be missing, so pin the
        // ordinary case too — otherwise "never sorts at all" would also produce
        // a string, just a different wrong one.
        let unsorted = vec![
            ("prefix".to_owned(), "x".to_owned()),
            ("list-type".to_owned(), "2".to_owned()),
        ];
        assert_eq!(canonical_query(&unsorted), "list-type=2&prefix=x");
    }

    #[test]
    fn the_secret_key_is_not_in_the_debug_output() {
        // The support bundle formats config structs, and operators attach it to
        // public issues.
        let creds = Credentials {
            access_key: "AKIAIOSFODNN7EXAMPLE".to_owned(),
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_owned(),
        };
        let shown = format!("{creds:?}");
        assert!(
            !shown.contains("wJalrXUtnFEMI"),
            "the secret key leaked into Debug output: {shown}"
        );
        assert!(
            shown.contains("AKIAIOSFODNN7EXAMPLE"),
            "the access key should still be visible for diagnostics: {shown}"
        );
    }
}
