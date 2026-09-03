//! An S3-compatible object store as a backup destination.
//!
//! Three requests — `PUT`, `GET ?list-type=2`, `DELETE` — signed by
//! [`super::sigv4`] and sent with the `reqwest` client the rest of this crate
//! already uses. That is enough for AWS S3, Backblaze B2's S3 API, Cloudflare
//! R2, Wasabi, `MinIO`, Ceph RGW and Garage, which between them cover everything
//! an operator is likely to point a Raspberry Pi at.
//!
//! # Addressing
//!
//! Two ways to name a bucket, and getting it wrong is the first thing that
//! fails:
//!
//! * **virtual-host**: `https://bucket.s3.eu-west-2.amazonaws.com/key` — what
//!   AWS requires for buckets created after September 2020.
//! * **path**: `https://minio.example.net:9000/bucket/key` — what every
//!   self-hosted implementation speaks, and what AWS still accepts for older
//!   buckets.
//!
//! [`Addressing::for_endpoint`] picks by endpoint host so the common cases need
//! no configuration, and the operator can override it when the guess is wrong.
//!
//! # What is deliberately not here
//!
//! **Multipart upload.** A station database is tens to hundreds of megabytes;
//! the single-`PUT` limit is 5 GiB. [`S3Target::put_object`] refuses above that
//! with an error that says so, rather than carrying the state machine — and the
//! resume logic, and the orphaned-part cleanup — for a case no station reaches.
//!
//! **Retries.** The caller owns them, because "retry the upload" and "retry the
//! delete" have different safety arguments and the caller is the one that knows
//! which it is doing.

use std::path::Path;
use std::time::Duration;

use super::sigv4::{self, Credentials, Request, hex};

/// Largest object a single `PUT` can carry, per the S3 API.
pub const MAX_SINGLE_PUT: u64 = 5 * 1024 * 1024 * 1024;

/// How long to wait for the TCP connection and TLS handshake.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the transfer may make **no progress at all** before it is dead.
///
/// Still no overall request timeout, and for the reason the previous comment
/// gave: a station on a rural uplink can legitimately spend an hour on one
/// upload, and a deadline that killed it would turn a slow link into a station
/// with no offsite backups at all. That half was right.
///
/// What the previous comment got wrong was the sentence after it — "a wedged
/// connection is caught by [`CONNECT_TIMEOUT`] instead". It is not.
/// `connect_timeout` bounds the connect and handshake only; a socket that
/// *establishes* and then stalls part-way through the exchange is not bounded
/// by it at all, which a probe against a server that sends headers, one byte,
/// and then holds confirmed by hanging past 45 seconds. That is the ordinary
/// 4G failure and the ordinary behaviour of a middlebox that has lost the far
/// side.
///
/// It mattered far beyond a missed upload: `run_offsite` is awaited inline in
/// the single sequential maintenance loop, so one wedged socket stopped the
/// daily integrity check, `VACUUM`, the local backup and every retention job
/// for the life of the process — with the `warn!` sitting on an error path that
/// was never reached.
///
/// A *read* timeout is the right instrument because it bounds inactivity rather
/// than duration: it resets on every successful read, so a slow-but-progressing
/// transfer is untouched however long it takes, while one that has gone quiet
/// for two minutes is called dead. Two minutes is generous for the gap between
/// the last byte of a `PUT` body and the first byte of the store's response,
/// which is the longest legitimate silence in this protocol.
pub const READ_TIMEOUT: Duration = Duration::from_secs(120);

/// How a bucket is addressed in the URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Addressing {
    /// `https://bucket.endpoint/key`.
    VirtualHost,
    /// `https://endpoint/bucket/key`.
    Path,
}

impl Addressing {
    /// The style an endpoint most likely wants.
    ///
    /// AWS deprecated path-style for buckets created after September 2020, so
    /// an `amazonaws.com` endpoint gets virtual-host. Everything else gets
    /// path-style, which every self-hosted implementation speaks and which some
    /// (`MinIO` in its default configuration, Garage) speak *only*.
    #[must_use]
    pub fn for_endpoint(host: &str) -> Self {
        let host = host.split(':').next().unwrap_or(host);
        if host.eq_ignore_ascii_case("amazonaws.com") || host.ends_with(".amazonaws.com") {
            Self::VirtualHost
        } else {
            Self::Path
        }
    }

    /// Parse an operator's setting: `auto`, `virtual`, or `path`.
    ///
    /// # Errors
    ///
    /// Returns the offending token.
    pub fn parse(token: &str, endpoint_host: &str) -> Result<Self, String> {
        match token.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Ok(Self::for_endpoint(endpoint_host)),
            "virtual" | "virtual-host" | "vhost" => Ok(Self::VirtualHost),
            "path" | "path-style" => Ok(Self::Path),
            other => Err(other.to_string()),
        }
    }
}

/// Where offsite backups go, and how to sign for them.
#[derive(Debug, Clone)]
pub struct S3Target {
    /// Scheme and authority, e.g. `https://s3.eu-west-2.amazonaws.com`. The
    /// scheme is kept because `MinIO` on a LAN is often plain HTTP.
    pub endpoint: String,
    /// Bucket name.
    pub bucket: String,
    /// Key prefix within the bucket, without a leading slash. May be empty.
    pub prefix: String,
    /// Region for the credential scope.
    pub region: String,
    /// Access key and secret.
    pub credentials: Credentials,
    /// How the bucket appears in the URL.
    pub addressing: Addressing,
}

/// One object as the store reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteObject {
    /// Full key, including any prefix.
    pub key: String,
    /// Size in bytes.
    pub size: u64,
    /// ISO-8601 last-modified stamp, exactly as the store returned it.
    ///
    /// Not parsed. Retention here orders by the timestamp embedded in the
    /// station's own filenames, which is the time the backup was *taken*;
    /// `LastModified` is the time it finished uploading, and on a station that
    /// spent a day catching up those two orders differ. Kept for display.
    pub last_modified: String,
}

/// What can go wrong talking to an object store.
#[derive(Debug)]
pub enum S3Error {
    /// The endpoint could not be parsed into a scheme and host.
    BadEndpoint(String),
    /// The object is larger than a single `PUT` can carry.
    TooLarge {
        /// The object's size in bytes.
        size: u64,
    },
    /// Local file could not be read.
    Io(std::io::Error),
    /// The request never completed.
    Transport(String),
    /// The store answered, and said no.
    Rejected {
        /// HTTP status.
        status: u16,
        /// `<Code>` from the XML error body, when there was one.
        code: String,
        /// `<Message>` from the XML error body, when there was one.
        message: String,
        /// Our canonical request, for a signature mismatch.
        ///
        /// A store that rejects a signature returns its own canonical request
        /// in the body; printing ours beside it turns `SignatureDoesNotMatch`
        /// from a guess into a diff.
        canonical_request: Option<String>,
    },
    /// The store answered with something this cannot read.
    BadResponse(String),
}

impl std::fmt::Display for S3Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadEndpoint(e) => write!(f, "endpoint is not a usable URL: {e}"),
            Self::TooLarge { size } => write!(
                f,
                "the backup is {size} bytes; a single PUT carries at most {MAX_SINGLE_PUT}. \
                 Prune the database or use a destination that takes larger objects"
            ),
            Self::Io(e) => write!(f, "reading the local backup: {e}"),
            Self::Transport(e) => write!(f, "request failed: {e}"),
            Self::Rejected {
                status,
                code,
                message,
                ..
            } if !code.is_empty() => write!(f, "store returned {status} {code}: {message}"),
            Self::Rejected { status, .. } => write!(f, "store returned HTTP {status}"),
            Self::BadResponse(e) => write!(f, "could not read the store's answer: {e}"),
        }
    }
}

impl std::error::Error for S3Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for S3Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// A request's URL and the pieces the signature needs, kept together.
///
/// Built by one function so the URL and the canonical path cannot disagree,
/// which is the single most common way a hand-written S3 client fails on
/// exactly the keys that contain a space.
#[derive(Debug, Clone)]
pub struct Addressed {
    /// Full request URL, percent-encoded.
    pub url: String,
    /// Host, with port when the endpoint carries one.
    pub host: String,
    /// Raw (unencoded) path, for the signer.
    pub path: String,
}

impl S3Target {
    /// The full key for a backup file name, with the configured prefix applied.
    #[must_use]
    pub fn key_for(&self, name: &str) -> String {
        let prefix = self.prefix.trim_matches('/');
        if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        }
    }

    /// Where a key lives, and what the signature covers.
    ///
    /// # Errors
    ///
    /// [`S3Error::BadEndpoint`] if the endpoint has no `://` or no host.
    pub fn address(&self, key: &str, query: &[(String, String)]) -> Result<Addressed, S3Error> {
        let (scheme, authority) = self
            .endpoint
            .split_once("://")
            .ok_or_else(|| S3Error::BadEndpoint(format!("{} has no scheme", self.endpoint)))?;
        let authority = authority.trim_end_matches('/');
        if authority.is_empty() {
            return Err(S3Error::BadEndpoint(format!(
                "{} has no host",
                self.endpoint
            )));
        }

        let (host, path) = match self.addressing {
            Addressing::VirtualHost => (format!("{}.{authority}", self.bucket), format!("/{key}")),
            Addressing::Path => (
                authority.to_owned(),
                if key.is_empty() {
                    format!("/{}", self.bucket)
                } else {
                    format!("/{}/{key}", self.bucket)
                },
            ),
        };

        let mut url = format!("{scheme}://{host}{}", sigv4::encode_path(&path));
        if !query.is_empty() {
            url.push('?');
            url.push_str(&sigv4::canonical_query(query));
        }
        Ok(Addressed { url, host, path })
    }

    /// Sign a request and return the headers to send with it.
    fn signed_headers(
        &self,
        method: &str,
        addressed: &Addressed,
        query: &[(String, String)],
        payload_sha256: &str,
        now: &str,
    ) -> sigv4::Signed {
        sigv4::sign(
            &self.credentials,
            &Request {
                method,
                host: &addressed.host,
                path: &addressed.path,
                query,
                payload_sha256,
                extra_headers: &[],
                region: &self.region,
                timestamp: now,
            },
        )
    }
}

/// The `x-amz-date` stamp for right now.
#[must_use]
pub fn timestamp_now() -> String {
    timestamp_at(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
}

/// `YYYYMMDDTHHMMSSZ` for a Unix timestamp.
///
/// Hand-rolled rather than pulled from `chrono`/`time`, for the same reason the
/// rest of this workspace formats its own dates: the civil-date conversion is
/// already here, in `birdnet_core::civil`, and this crate does not depend on it.
/// Twelve lines against a dependency edge.
#[must_use]
pub fn timestamp_at(unix_secs: u64) -> String {
    let days = unix_secs / 86_400;
    let secs = unix_secs % 86_400;
    let (y, m, d) = civil_from_days(i64::try_from(days).unwrap_or(0));
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Howard Hinnant's `civil_from_days`, for the date half of the stamp.
///
/// The same algorithm as `birdnet_core::civil::civil_from_days`; duplicated
/// rather than depended on because `birdnet-integrations` does not otherwise
/// need `birdnet-core`, and `timestamp_round_trips_against_known_dates` pins it
/// against dates checked by hand.
#[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)]
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        i32::try_from(y + i64::from(m <= 2)).unwrap_or(0),
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

/// Read a whole file and return its hex SHA-256, without holding it in memory.
///
/// # Errors
///
/// Any read failure.
pub fn file_sha256(path: &Path) -> Result<(String, u64), std::io::Error> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        sha2::Digest::update(&mut hasher, &buf[..n]);
        total += n as u64;
    }
    Ok((hex(&sha2::Digest::finalize(hasher)), total))
}

/// Extract the text of the first `<tag>` in `xml`, if any.
///
/// A five-line scanner rather than an XML parser: the two documents this reads
/// — `ListBucketResult` and `Error` — are flat, machine-generated, and never
/// carry attributes or namespaced names on the elements wanted here. Anything
/// it cannot find comes back as `None` and is reported as
/// [`S3Error::BadResponse`], so a store that answers in a shape this does not
/// expect is a legible error rather than a silent empty listing.
#[must_use]
pub fn xml_text<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(&xml[start..end])
}

/// Every `<Contents>` block in a `ListBucketResult`.
///
/// # Errors
///
/// [`S3Error::BadResponse`] if a block is missing `<Key>` or has a `<Size>`
/// that is not a number — a listing this cannot read must not be mistaken for
/// an empty bucket, because retention would then decide there is nothing to
/// keep and nothing to prune.
pub fn parse_listing(xml: &str) -> Result<(Vec<RemoteObject>, Option<String>), S3Error> {
    if !xml.contains("<ListBucketResult") {
        return Err(S3Error::BadResponse(format!(
            "expected a ListBucketResult, got {} bytes starting {:?}",
            xml.len(),
            &xml[..xml.len().min(80)]
        )));
    }
    let mut out = Vec::new();
    for block in xml.split("<Contents>").skip(1) {
        let block = block.split("</Contents>").next().unwrap_or(block);
        let key = xml_text(block, "Key")
            .ok_or_else(|| S3Error::BadResponse("a <Contents> block has no <Key>".to_owned()))?;
        let size: u64 = xml_text(block, "Size")
            .unwrap_or("0")
            .trim()
            .parse()
            .map_err(|_| S3Error::BadResponse(format!("<Size> for {key} is not a number")))?;
        out.push(RemoteObject {
            key: unescape_xml(key),
            size,
            last_modified: xml_text(block, "LastModified").unwrap_or("").to_owned(),
        });
    }

    // Only follow a continuation token when the store says the listing was
    // truncated. Some stores send the token unconditionally, and following it
    // forever is how a retention pass turns into an infinite loop.
    let truncated = xml_text(xml, "IsTruncated").is_some_and(|v| v.trim() == "true");
    let token = if truncated {
        xml_text(xml, "NextContinuationToken").map(unescape_xml)
    } else {
        None
    };
    Ok((out, token))
}

/// The five XML entities, expanded.
///
/// S3 escapes `&` in keys, and a key containing `&amp;` that came back
/// unescaped would be deleted under the wrong name — or, worse, not deleted,
/// leaving retention convinced it had pruned.
#[must_use]
pub fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Pull `<Code>` and `<Message>` out of an S3 error body.
#[must_use]
pub fn parse_error(xml: &str) -> (String, String) {
    (
        xml_text(xml, "Code").unwrap_or("").to_owned(),
        xml_text(xml, "Message").unwrap_or("").to_owned(),
    )
}

/// Build the HTTP client the target uses.
///
/// # Errors
///
/// [`S3Error::Transport`] if the TLS stack will not initialise.
pub fn client() -> Result<reqwest::Client, S3Error> {
    client_with_timeouts(CONNECT_TIMEOUT, READ_TIMEOUT)
}

/// [`client`] with the timeouts injected.
///
/// Exists so a test can watch the stall detector fire in two seconds rather
/// than in two minutes. Production always goes through [`client`], which is a
/// one-line delegation, so the two cannot drift apart.
///
/// # Errors
///
/// [`S3Error::Transport`] if the TLS stack will not initialise.
pub fn client_with_timeouts(connect: Duration, read: Duration) -> Result<reqwest::Client, S3Error> {
    reqwest::Client::builder()
        .connect_timeout(connect)
        .read_timeout(read)
        .user_agent(concat!("BirdNet-Behavior/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| S3Error::Transport(e.to_string()))
}

impl S3Target {
    /// Upload a local file under `name` (the configured prefix is applied).
    ///
    /// # Errors
    ///
    /// [`S3Error::TooLarge`] above [`MAX_SINGLE_PUT`], [`S3Error::Io`] if the
    /// file cannot be read, [`S3Error::Transport`] if the request does not
    /// complete, and [`S3Error::Rejected`] if the store refuses it.
    pub async fn put_object(
        &self,
        client: &reqwest::Client,
        name: &str,
        path: &Path,
        digest: &str,
        len: u64,
    ) -> Result<String, S3Error> {
        if len > MAX_SINGLE_PUT {
            return Err(S3Error::TooLarge { size: len });
        }
        let key = self.key_for(name);
        let addressed = self.address(&key, &[])?;
        let now = timestamp_now();
        let signed = self.signed_headers("PUT", &addressed, &[], digest, &now);

        let file = tokio::fs::File::open(path).await?;
        let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(file));

        let mut req = client
            .put(&addressed.url)
            .header("authorization", &signed.authorization)
            .header("content-length", len.to_string())
            .body(body);
        for (name, value) in &signed.headers {
            // `host` is set by the transport from the URL; setting it again is
            // rejected by hyper as a duplicate.
            if name != "host" {
                req = req.header(name.as_str(), value.as_str());
            }
        }

        let response = req
            .send()
            .await
            .map_err(|e| S3Error::Transport(e.to_string()))?;
        check(response, Some(&signed.canonical_request)).await?;
        Ok(key)
    }

    /// Every object under the configured prefix, following continuations.
    ///
    /// # Errors
    ///
    /// As [`S3Target::put_object`], plus [`S3Error::BadResponse`] when the
    /// listing cannot be parsed — which is deliberately *not* reported as an
    /// empty bucket.
    pub async fn list_objects(
        &self,
        client: &reqwest::Client,
    ) -> Result<Vec<RemoteObject>, S3Error> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        // A station keeps a handful of backups; a thousand pages means
        // something is wrong with the store's paging, not with the station.
        for _ in 0..1000 {
            let mut query = vec![
                ("list-type".to_owned(), "2".to_owned()),
                ("max-keys".to_owned(), "1000".to_owned()),
            ];
            let prefix = self.prefix.trim_matches('/');
            if !prefix.is_empty() {
                query.push(("prefix".to_owned(), format!("{prefix}/")));
            }
            if let Some(t) = &token {
                query.push(("continuation-token".to_owned(), t.clone()));
            }

            let addressed = self.address("", &query)?;
            let now = timestamp_now();
            let signed =
                self.signed_headers("GET", &addressed, &query, sigv4::EMPTY_PAYLOAD_SHA256, &now);

            let mut req = client
                .get(&addressed.url)
                .header("authorization", &signed.authorization);
            for (name, value) in &signed.headers {
                if name != "host" {
                    req = req.header(name.as_str(), value.as_str());
                }
            }
            let response = req
                .send()
                .await
                .map_err(|e| S3Error::Transport(e.to_string()))?;
            let body = check(response, Some(&signed.canonical_request)).await?;

            let (mut page, next) = parse_listing(&body)?;
            out.append(&mut page);
            match next {
                Some(t) => token = Some(t),
                None => return Ok(out),
            }
        }
        Err(S3Error::BadResponse(
            "the store kept returning continuation tokens past 1000 pages".to_owned(),
        ))
    }

    /// Delete one object by its full key.
    ///
    /// # Errors
    ///
    /// As [`S3Target::put_object`].
    pub async fn delete_object(&self, client: &reqwest::Client, key: &str) -> Result<(), S3Error> {
        let addressed = self.address(key, &[])?;
        let now = timestamp_now();
        let signed =
            self.signed_headers("DELETE", &addressed, &[], sigv4::EMPTY_PAYLOAD_SHA256, &now);
        let mut req = client
            .delete(&addressed.url)
            .header("authorization", &signed.authorization);
        for (name, value) in &signed.headers {
            if name != "host" {
                req = req.header(name.as_str(), value.as_str());
            }
        }
        let response = req
            .send()
            .await
            .map_err(|e| S3Error::Transport(e.to_string()))?;
        check(response, Some(&signed.canonical_request)).await?;
        Ok(())
    }
}

/// Turn a non-2xx response into an [`S3Error::Rejected`]; return the body otherwise.
async fn check(
    response: reqwest::Response,
    canonical_request: Option<&str>,
) -> Result<String, S3Error> {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if status.is_success() {
        return Ok(body);
    }
    let (code, message) = parse_error(&body);
    Err(S3Error::Rejected {
        status: status.as_u16(),
        code: code.clone(),
        message,
        // Only worth carrying for the one error it helps with; on a 404 it is
        // noise in the log.
        canonical_request: (code == "SignatureDoesNotMatch")
            .then(|| canonical_request.unwrap_or("").to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(endpoint: &str, addressing: Addressing) -> S3Target {
        S3Target {
            endpoint: endpoint.to_owned(),
            bucket: "birdnet".to_owned(),
            prefix: "stations/pi-1".to_owned(),
            region: "eu-west-2".to_owned(),
            credentials: Credentials {
                access_key: "AKIAIOSFODNN7EXAMPLE".to_owned(),
                secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_owned(),
            },
            addressing,
        }
    }

    #[test]
    fn addressing_is_guessed_from_the_endpoint_and_both_guesses_are_pinned() {
        // AWS deprecated path-style for new buckets; every self-hosted store
        // speaks path-style and some speak only that. A wrong guess here is the
        // first thing an operator hits.
        assert_eq!(
            Addressing::for_endpoint("s3.eu-west-2.amazonaws.com"),
            Addressing::VirtualHost
        );
        assert_eq!(
            Addressing::for_endpoint("minio.example.net:9000"),
            Addressing::Path
        );
        // The counterpart that a naive `contains("amazonaws")` would get wrong.
        assert_eq!(
            Addressing::for_endpoint("s3.amazonaws.com.evil.example"),
            Addressing::Path,
            "a lookalike host must not be treated as AWS"
        );
        assert_eq!(
            Addressing::parse("path", "s3.eu-west-2.amazonaws.com").unwrap(),
            Addressing::Path,
            "an explicit setting must beat the guess"
        );
        assert_eq!(
            Addressing::parse("auto", "s3.eu-west-2.amazonaws.com").unwrap(),
            Addressing::VirtualHost
        );
        assert_eq!(Addressing::parse("sideways", "x").unwrap_err(), "sideways");
    }

    #[test]
    fn the_url_and_the_signed_path_always_describe_the_same_object() {
        // The invariant `address` exists to hold. A URL and a canonical path
        // built by two different pieces of code is how an S3 client comes to
        // work for every key except the ones with a space in them.
        for (endpoint, addressing, expect_host, expect_url) in [
            (
                "https://s3.eu-west-2.amazonaws.com",
                Addressing::VirtualHost,
                "birdnet.s3.eu-west-2.amazonaws.com",
                "https://birdnet.s3.eu-west-2.amazonaws.com/stations/pi-1/a%20b.bnb",
            ),
            (
                "http://minio.example.net:9000",
                Addressing::Path,
                "minio.example.net:9000",
                "http://minio.example.net:9000/birdnet/stations/pi-1/a%20b.bnb",
            ),
        ] {
            let t = target(endpoint, addressing);
            let key = t.key_for("a b.bnb");
            let a = t.address(&key, &[]).expect("address");
            assert_eq!(a.host, expect_host);
            assert_eq!(a.url, expect_url);
            assert_eq!(
                a.url,
                format!(
                    "{}://{}{}",
                    endpoint.split("://").next().unwrap(),
                    a.host,
                    sigv4::encode_path(&a.path)
                ),
                "the URL and the signed path disagree"
            );
        }
    }

    #[test]
    fn a_trailing_slash_or_missing_scheme_on_the_endpoint_is_handled_or_named() {
        let mut t = target("https://minio.example.net:9000/", Addressing::Path);
        let a = t
            .address("k", &[])
            .expect("a trailing slash is not an error");
        assert_eq!(a.url, "https://minio.example.net:9000/birdnet/k");

        t.endpoint = "minio.example.net".to_owned();
        assert!(matches!(t.address("k", &[]), Err(S3Error::BadEndpoint(_))));
        t.endpoint = "https://".to_owned();
        assert!(matches!(t.address("k", &[]), Err(S3Error::BadEndpoint(_))));
    }

    #[test]
    fn an_empty_prefix_does_not_produce_a_leading_slash_in_the_key() {
        // `//name` is a *different key* from `name` in S3, and a station that
        // wrote under one and pruned under the other would accumulate forever.
        let mut t = target("https://x.example", Addressing::Path);
        t.prefix = String::new();
        assert_eq!(t.key_for("birds.db.backup.1.bnb"), "birds.db.backup.1.bnb");
        t.prefix = "/a/b/".to_owned();
        assert_eq!(t.key_for("c.bnb"), "a/b/c.bnb");
    }

    #[test]
    fn a_listing_is_parsed_and_a_truncated_one_reports_its_token() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>birdnet</Name><Prefix>stations/pi-1/</Prefix>
  <KeyCount>2</KeyCount><MaxKeys>1000</MaxKeys><IsTruncated>true</IsTruncated>
  <NextContinuationToken>1/abc+def==</NextContinuationToken>
  <Contents><Key>stations/pi-1/birds.db.backup.100.bnb</Key>
    <LastModified>2026-03-01T04:00:00.000Z</LastModified><Size>1048576</Size>
    <StorageClass>STANDARD</StorageClass></Contents>
  <Contents><Key>stations/pi-1/a&amp;b.bnb</Key>
    <LastModified>2026-03-02T04:00:00.000Z</LastModified><Size>7</Size>
    <StorageClass>STANDARD</StorageClass></Contents>
</ListBucketResult>"#;
        let (objects, token) = parse_listing(xml).expect("parse");
        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0].key, "stations/pi-1/birds.db.backup.100.bnb");
        assert_eq!(objects[0].size, 1_048_576);
        assert_eq!(objects[0].last_modified, "2026-03-01T04:00:00.000Z");
        assert_eq!(
            objects[1].key, "stations/pi-1/a&b.bnb",
            "an escaped ampersand in a key must come back unescaped, or the \
             delete would name a different object"
        );
        assert_eq!(token.as_deref(), Some("1/abc+def=="));
    }

    #[test]
    fn a_continuation_token_is_ignored_unless_the_listing_says_it_is_truncated() {
        // Some stores send the token unconditionally. Following it regardless
        // is an infinite retention loop.
        let xml = "<ListBucketResult><IsTruncated>false</IsTruncated>\
                   <NextContinuationToken>tok</NextContinuationToken></ListBucketResult>";
        let (objects, token) = parse_listing(xml).expect("parse");
        assert!(objects.is_empty());
        assert!(
            token.is_none(),
            "a non-truncated listing must not be followed"
        );
    }

    #[test]
    fn an_unreadable_listing_is_an_error_and_not_an_empty_bucket() {
        // The distinction retention depends on: "there is nothing there" and
        // "I could not tell" must not be the same answer, or a store that
        // starts answering in HTML deletes nothing and reports success forever.
        assert!(matches!(
            parse_listing("<html><body>502 Bad Gateway</body></html>"),
            Err(S3Error::BadResponse(_))
        ));
        assert!(matches!(
            parse_listing(
                "<ListBucketResult><Contents><Size>1</Size></Contents></ListBucketResult>"
            ),
            Err(S3Error::BadResponse(_))
        ));
        assert!(matches!(
            parse_listing(
                "<ListBucketResult><Contents><Key>k</Key><Size>lots</Size></Contents></ListBucketResult>"
            ),
            Err(S3Error::BadResponse(_))
        ));
        // Counterpart: a genuinely empty bucket is not an error.
        let (objects, token) =
            parse_listing("<ListBucketResult><KeyCount>0</KeyCount></ListBucketResult>")
                .expect("an empty listing is a valid answer");
        assert!(objects.is_empty() && token.is_none());
    }

    #[test]
    fn an_error_body_is_read_for_its_code_and_message() {
        let xml = "<?xml version=\"1.0\"?><Error><Code>SignatureDoesNotMatch</Code>\
                   <Message>The request signature we calculated does not match</Message>\
                   <RequestId>ABC</RequestId></Error>";
        let (code, message) = parse_error(xml);
        assert_eq!(code, "SignatureDoesNotMatch");
        assert!(message.starts_with("The request signature"));
        // And a body with no XML at all does not panic or invent a code.
        assert_eq!(
            parse_error("gateway timeout"),
            (String::new(), String::new())
        );
    }

    #[test]
    fn timestamps_round_trip_against_dates_checked_by_hand() {
        // The stamp is half the credential scope, so an off-by-one day makes
        // every request fail at midnight UTC and work again in the morning —
        // the kind of bug that gets diagnosed as "flaky network".
        assert_eq!(timestamp_at(0), "19700101T000000Z");
        // 2013-05-24T00:00:00Z, the epoch second AWS's own example uses.
        assert_eq!(timestamp_at(1_369_353_600), "20130524T000000Z");
        // A leap day, and one second either side of midnight.
        assert_eq!(timestamp_at(1_709_164_800), "20240229T000000Z");
        assert_eq!(timestamp_at(1_709_164_799), "20240228T235959Z");
        assert_eq!(timestamp_at(1_709_251_199), "20240229T235959Z");
        assert_eq!(timestamp_at(1_709_251_200), "20240301T000000Z");
    }

    #[test]
    fn a_files_digest_and_length_match_a_one_shot_hash() {
        // `file_sha256` reads in 256 KiB blocks; a block-boundary bug would
        // give a wrong digest and every upload would be rejected. Compared
        // against hashing the same bytes in one go.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("blob");
        // Deliberately not a multiple of the block size.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bytes: Vec<u8> = (0..(256_usize * 1024 + 12345))
            .map(|i| (i % 251) as u8)
            .collect();
        std::fs::write(&path, &bytes).expect("write");

        let (digest, len) = file_sha256(&path).expect("hash");
        assert_eq!(len, bytes.len() as u64);
        assert_eq!(digest, hex(&sigv4::sha256(&bytes)));

        // And an empty file is the documented constant, which is what every
        // bodyless request signs with.
        let empty = dir.path().join("empty");
        std::fs::write(&empty, b"").expect("write");
        assert_eq!(
            file_sha256(&empty).expect("hash").0,
            sigv4::EMPTY_PAYLOAD_SHA256
        );
    }
}
