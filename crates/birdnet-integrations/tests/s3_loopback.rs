//! The S3 client against a store that checks its work, over a real socket.
//!
//! # What this catches that the vector test cannot
//!
//! `sigv4_vectors.rs` proves the *signature* is right: given a request, the
//! bytes match `botocore`. It says nothing about whether the client then sends
//! that request. Those are different failures, and the second is the one that
//! actually happens — a header signed but not set, a header set but not signed,
//! a body whose hash is not the one in `x-amz-content-sha256`, a query string
//! built one way for the URL and another for the signature.
//!
//! So this stands up a minimal HTTP/1.1 server on a loopback port that behaves
//! like a strict S3-compatible store: it recomputes the signature from what
//! arrived on the wire and rejects a mismatch with `403 SignatureDoesNotMatch`,
//! exactly as `MinIO` would, and it hashes the body it received and rejects a
//! disagreement with the declared payload hash.
//!
//! Deliberately hand-rolled HTTP rather than a framework: the point is to
//! inspect the literal bytes the client put on the socket, and a framework
//! would normalise some of them away. It also keeps this crate's dev-dependency
//! list where it is.
//!
//! # What it deliberately does not check
//!
//! Percent-encoding. The server decodes the request target back to a raw path
//! before re-signing, so an encoder that was wrong in a self-consistent way
//! would still pass here. That is the vector test's job, and splitting them
//! keeps each one's failure message pointed at one thing.

use std::collections::BTreeMap;
use std::sync::Arc;

use birdnet_integrations::offsite::s3::{Addressing, S3Error, S3Target, file_sha256};
use birdnet_integrations::offsite::sigv4::{self, Credentials};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

/// One request as it arrived.
#[derive(Debug, Clone)]
struct Wire {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

/// What the fake store did with a request, for the test to assert on.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    target: String,
    body_len: usize,
    body_sha256: String,
}

/// A store that verifies signatures and records what it was asked for.
struct FakeStore {
    seen: std::sync::Mutex<Vec<Seen>>,
    /// Bodies to serve for `GET`, in order; each is used once.
    listings: std::sync::Mutex<Vec<String>>,
}

impl FakeStore {
    const fn new(listings: Vec<String>) -> Self {
        Self {
            seen: std::sync::Mutex::new(Vec::new()),
            listings: std::sync::Mutex::new(listings),
        }
    }
}

/// Percent-decode a request target's path.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Rebuild the request from the wire and check the client's signature.
///
/// This is what `MinIO` does: split the target, take the signed headers the
/// client named, and re-derive. A header the client signed but did not send is
/// absent here and the signature cannot match; a header it sent but did not
/// sign is not in `SignedHeaders` and is ignored — which is the asymmetry that
/// makes "signed but not sent" the failure that shows up in production.
fn signature_matches(wire: &Wire) -> Result<(), String> {
    let auth = wire
        .headers
        .get("authorization")
        .ok_or("no Authorization header")?;
    let signed_headers = auth
        .split("SignedHeaders=")
        .nth(1)
        .and_then(|s| s.split(',').next())
        .ok_or("Authorization has no SignedHeaders")?
        .trim();
    let scope = auth
        .split("Credential=")
        .nth(1)
        .and_then(|s| s.split(',').next())
        .ok_or("Authorization has no Credential")?;
    let region = scope.split('/').nth(2).ok_or("no region in scope")?;

    let (raw_target, raw_query) = wire.target.split_once('?').unwrap_or((&wire.target, ""));
    let path = percent_decode(raw_target);
    let query: Vec<(String, String)> = if raw_query.is_empty() {
        Vec::new()
    } else {
        raw_query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (percent_decode(k), percent_decode(v)))
            .collect()
    };

    let host = wire.headers.get("host").ok_or("no Host header")?;
    let payload = wire
        .headers
        .get("x-amz-content-sha256")
        .ok_or("no x-amz-content-sha256")?;
    let stamp = wire.headers.get("x-amz-date").ok_or("no x-amz-date")?;

    // Anything the client signed beyond the three that are always present.
    let extra: Vec<(String, String)> = signed_headers
        .split(';')
        .filter(|h| !matches!(*h, "host" | "x-amz-content-sha256" | "x-amz-date"))
        .map(|h| {
            let v = wire.headers.get(h).cloned().unwrap_or_default();
            (h.to_owned(), v)
        })
        .collect();

    let creds = Credentials {
        access_key: ACCESS_KEY.to_owned(),
        secret_key: SECRET_KEY.to_owned(),
    };
    let expected = sigv4::sign(
        &creds,
        &sigv4::Request {
            method: &wire.method,
            host,
            path: &path,
            query: &query,
            payload_sha256: payload,
            extra_headers: &extra,
            region,
            timestamp: stamp,
        },
    );
    if &expected.authorization == auth {
        Ok(())
    } else {
        Err(format!(
            "signature mismatch\n  client sent: {auth}\n  store expected: {}\n  canonical:\n{}",
            expected.authorization, expected.canonical_request
        ))
    }
}

/// Serve one connection.
async fn serve(mut socket: tokio::net::TcpStream, store: Arc<FakeStore>) {
    let mut buf = Vec::new();
    // Read to the end of the headers.
    let head_end = loop {
        let mut chunk = [0u8; 8192];
        let Ok(n) = socket.read(&mut chunk).await else {
            return;
        };
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > 1 << 20 {
            return;
        }
    };

    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();

    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }

    // Read the declared body.
    let want: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[head_end..].to_vec();
    while body.len() < want {
        let mut chunk = [0u8; 8192];
        let Ok(n) = socket.read(&mut chunk).await else {
            break;
        };
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }

    let wire = Wire {
        method: method.clone(),
        target: target.clone(),
        headers,
        body,
    };

    let reply = |status: &str, body: &str| -> Vec<u8> {
        format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    };

    let response = if let Err(why) = signature_matches(&wire) {
        reply(
            "403 Forbidden",
            &format!("<Error><Code>SignatureDoesNotMatch</Code><Message>{why}</Message></Error>"),
        )
    } else {
        let declared = wire
            .headers
            .get("x-amz-content-sha256")
            .cloned()
            .unwrap_or_default();
        let actual = sigv4::hex(&sigv4::sha256(&wire.body));
        if declared == actual {
            store.seen.lock().unwrap().push(Seen {
                method: wire.method.clone(),
                target: wire.target.clone(),
                body_len: wire.body.len(),
                body_sha256: actual,
            });
            match wire.method.as_str() {
                "GET" => {
                    let next = store.listings.lock().unwrap().remove(0);
                    reply("200 OK", &next)
                }
                "PUT" => reply("200 OK", ""),
                "DELETE" => reply("204 No Content", ""),
                _ => reply("405 Method Not Allowed", ""),
            }
        } else {
            reply(
                "400 Bad Request",
                "<Error><Code>XAmzContentSHA256Mismatch</Code>\
                 <Message>the body does not hash to the declared value</Message></Error>",
            )
        }
    };

    let _ = socket.write_all(&response).await;
    let _ = socket.flush().await;
}

/// Start the store; returns its endpoint and a handle to what it saw.
async fn start(listings: Vec<String>) -> (String, Arc<FakeStore>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let store = Arc::new(FakeStore::new(listings));
    let for_task = Arc::clone(&store);
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            let store = Arc::clone(&for_task);
            tokio::spawn(serve(socket, store));
        }
    });
    (format!("http://{addr}"), store)
}

fn target(endpoint: &str) -> S3Target {
    S3Target {
        endpoint: endpoint.to_owned(),
        bucket: "birdnet".to_owned(),
        prefix: "stations/pi-1".to_owned(),
        region: "eu-west-2".to_owned(),
        credentials: Credentials {
            access_key: ACCESS_KEY.to_owned(),
            secret_key: SECRET_KEY.to_owned(),
        },
        // Path style: a virtual-host URL would need `birdnet.127.0.0.1` to
        // resolve, which is exactly why self-hosted stores use path style.
        addressing: Addressing::Path,
    }
}

#[tokio::test]
async fn an_upload_arrives_intact_and_signed() {
    let (endpoint, store) = start(Vec::new()).await;
    let t = target(&endpoint);
    let client = birdnet_integrations::offsite::s3::client().expect("client");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("birds.db.backup.1733400000.bnb");
    // Larger than one read of the streaming body, so a truncated stream shows.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let bytes: Vec<u8> = (0..300_000_usize).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &bytes).expect("write");
    let (digest, len) = file_sha256(&path).expect("hash");

    let key = t
        .put_object(
            &client,
            "birds.db.backup.1733400000.bnb",
            &path,
            &digest,
            len,
        )
        .await
        .expect("the store must accept a correctly signed upload");
    assert_eq!(key, "stations/pi-1/birds.db.backup.1733400000.bnb");

    let seen = store.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "expected exactly one request: {seen:?}");
    assert_eq!(seen[0].method, "PUT");
    assert_eq!(
        seen[0].target, "/birdnet/stations/pi-1/birds.db.backup.1733400000.bnb",
        "the object landed under the wrong key"
    );
    assert_eq!(
        seen[0].body_len,
        bytes.len(),
        "the streamed body was truncated"
    );
    assert_eq!(seen[0].body_sha256, digest);
}

#[tokio::test]
async fn a_wrong_secret_is_rejected_the_way_a_real_store_rejects_it() {
    // The counterpart to the test above: it would pass against a store that
    // accepted anything, so prove the store is actually checking.
    let (endpoint, _store) = start(Vec::new()).await;
    let mut t = target(&endpoint);
    t.credentials.secret_key = "not the right secret at all".to_owned();
    let client = birdnet_integrations::offsite::s3::client().expect("client");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("x.bnb");
    std::fs::write(&path, b"hello").expect("write");
    let (digest, len) = file_sha256(&path).expect("hash");

    let err = t
        .put_object(&client, "x.bnb", &path, &digest, len)
        .await
        .expect_err("a bad signature must not be reported as a successful upload");
    match err {
        S3Error::Rejected { status, code, .. } => {
            assert_eq!(status, 403);
            assert_eq!(code, "SignatureDoesNotMatch");
        }
        other => panic!("expected a rejection, got {other}"),
    }
}

#[tokio::test]
async fn a_declared_hash_that_does_not_match_the_body_is_caught() {
    // What `x-amz-content-sha256` is for. A client that signed one file and
    // streamed another would otherwise upload silent corruption.
    let (endpoint, _store) = start(Vec::new()).await;
    let t = target(&endpoint);
    let client = birdnet_integrations::offsite::s3::client().expect("client");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("x.bnb");
    std::fs::write(&path, b"the real bytes").expect("write");
    let wrong = sigv4::hex(&sigv4::sha256(b"different bytes entirely"));

    let err = t
        .put_object(&client, "x.bnb", &path, &wrong, 14)
        .await
        .expect_err("a mismatched payload hash must be rejected");
    match err {
        S3Error::Rejected { code, .. } => assert_eq!(code, "XAmzContentSHA256Mismatch"),
        other => panic!("expected a rejection, got {other}"),
    }
}

#[tokio::test]
async fn a_listing_follows_its_continuation_and_stops() {
    let page1 = "<ListBucketResult><IsTruncated>true</IsTruncated>\
        <NextContinuationToken>1/abc+def==</NextContinuationToken>\
        <Contents><Key>stations/pi-1/a.bnb</Key><Size>1</Size>\
        <LastModified>2026-01-01T00:00:00Z</LastModified></Contents></ListBucketResult>"
        .to_owned();
    let page2 = "<ListBucketResult><IsTruncated>false</IsTruncated>\
        <Contents><Key>stations/pi-1/b.bnb</Key><Size>2</Size>\
        <LastModified>2026-01-02T00:00:00Z</LastModified></Contents></ListBucketResult>"
        .to_owned();
    let (endpoint, store) = start(vec![page1, page2]).await;
    let t = target(&endpoint);
    let client = birdnet_integrations::offsite::s3::client().expect("client");

    let objects = t.list_objects(&client).await.expect("list");
    assert_eq!(
        objects.len(),
        2,
        "both pages must be collected: {objects:?}"
    );
    assert_eq!(objects[0].key, "stations/pi-1/a.bnb");
    assert_eq!(objects[1].key, "stations/pi-1/b.bnb");

    let seen = store.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "expected exactly two listing requests");
    assert!(
        seen[0].target.contains("list-type=2")
            && seen[0].target.contains("prefix=stations%2Fpi-1%2F"),
        "the first listing did not carry list-type and the prefix: {}",
        seen[0].target
    );
    assert!(
        !seen[0].target.contains("continuation-token"),
        "the first listing must not carry a token: {}",
        seen[0].target
    );
    assert!(
        seen[1]
            .target
            .contains("continuation-token=1%2Fabc%2Bdef%3D%3D"),
        "the second listing must carry the encoded token from page one: {}",
        seen[1].target
    );
}

#[tokio::test]
async fn a_delete_names_the_key_it_was_given() {
    let (endpoint, store) = start(Vec::new()).await;
    let t = target(&endpoint);
    let client = birdnet_integrations::offsite::s3::client().expect("client");

    t.delete_object(&client, "stations/pi-1/birds.db.backup.1.bnb")
        .await
        .expect("delete");

    let seen = store.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "DELETE");
    assert_eq!(
        seen[0].target, "/birdnet/stations/pi-1/birds.db.backup.1.bnb",
        "delete must address the full key it was handed, prefix included"
    );
}

#[tokio::test]
async fn an_object_too_large_for_one_put_is_refused_before_anything_is_sent() {
    // The store is never contacted, which is the point: a five-gigabyte upload
    // that is going to be rejected should not spend an hour of a station's
    // uplink first.
    let (endpoint, store) = start(Vec::new()).await;
    let t = target(&endpoint);
    let client = birdnet_integrations::offsite::s3::client().expect("client");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("x.bnb");
    std::fs::write(&path, b"pretend this is enormous").expect("write");

    let err = t
        .put_object(&client, "x.bnb", &path, "00", 6 * 1024 * 1024 * 1024)
        .await
        .expect_err("must refuse");
    assert!(matches!(err, S3Error::TooLarge { .. }), "got {err}");
    assert!(
        store.seen.lock().unwrap().is_empty(),
        "nothing should have been sent"
    );
}
