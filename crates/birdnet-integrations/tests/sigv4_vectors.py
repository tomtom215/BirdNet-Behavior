"""SigV4 reference vectors produced end-to-end by botocore's own S3 signer.

`add_auth` runs the whole flow — it adds `x-amz-date`, recomputes
`x-amz-content-sha256` from the body, builds the canonical request and emits the
`Authorization` header. That header is the vector: it is exactly the string the
Rust signer has to produce, byte for byte.

Paths are given RAW (unencoded); the URL is built by percent-encoding them the
way an S3 endpoint expects, which the Rust side must reproduce.
"""
import sys, json, datetime, hashlib
sys.path.insert(0, sys.argv[1])
from botocore.auth import S3SigV4Auth
from botocore.awsrequest import AWSRequest
from botocore.credentials import Credentials
from urllib.parse import quote

CREDS = Credentials("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY")
WHEN = datetime.datetime(2013, 5, 24, 0, 0, 0, tzinfo=datetime.timezone.utc)
STAMP = WHEN.strftime("%Y%m%dT%H%M%SZ")

# (name, method, host, raw_path, query, body bytes, region, extra headers)
CASES = [
    # (name, method, host, raw_path, query, body, region, extra headers)
    ("put_object_virtual_host", "PUT", "examplebucket.s3.amazonaws.com",
     "/test$file.text", {}, b"Welcome to Amazon S3.", "us-east-1", {}),
    ("get_object_virtual_host", "GET", "examplebucket.s3.amazonaws.com",
     "/test.txt", {}, b"", "us-east-1", {}),
    ("list_objects_v2_path_style_port", "GET", "minio.example.net:9000",
     "/birdnet", {"list-type": "2", "max-keys": "1000", "prefix": "stations/pi-1/"},
     b"", "us-east-1", {}),
    ("list_objects_v2_continuation", "GET", "s3.eu-central-1.wasabisys.com",
     "/mybucket", {"list-type": "2", "prefix": "b/", "continuation-token": "1/abc+def=="},
     b"", "eu-central-1", {}),
    ("delete_path_style_regional", "DELETE", "s3.eu-central-1.wasabisys.com",
     "/mybucket/backups/birds.db.backup.1733400000.bnb", {}, b"", "eu-central-1", {}),
    ("put_key_needing_encoding", "PUT", "storage.example.org",
     "/bkt/a b/c+d~e.bin", {}, b"\x00\x01\x02", "us-west-2", {}),
    # AWS's own published "Example: GET Object" from the S3 documentation, whose
    # signature is printed there. botocore reproduces it exactly, which is what
    # makes botocore usable as the reference for every other vector here.
    ("aws_published_get_object", "GET", "examplebucket.s3.amazonaws.com",
     "/test.txt", {}, b"", "us-east-1", {"Range": "bytes=0-9"}),
]

out = []
for name, method, host, raw_path, query, body, region, extra in CASES:
    url = "https://" + host + quote(raw_path, safe="/~")
    if query:
        url += "?" + "&".join(
            f"{quote(k, safe='-_.~')}={quote(v, safe='-_.~')}"
            for k, v in sorted(query.items())
        )
    req = AWSRequest(method=method, url=url, data=body, headers=dict(extra))
    signer = S3SigV4Auth(CREDS, "s3", region)
    req.context["payload_signing_enabled"] = True
    # botocore's own add_auth body with the clock pinned, so the vectors are
    # reproducible. Anything else here is botocore's code path unchanged.
    req.context["timestamp"] = STAMP
    signer._modify_request_before_signing(req)
    canonical = signer.canonical_request(req)
    sts = signer.string_to_sign(req, canonical)
    sig = signer.signature(sts, req)
    signer._inject_signature_to_request(req, sig)
    out.append({
        "name": name, "method": method, "host": host, "path": raw_path,
        "query": query, "region": region, "timestamp": STAMP,
        "extra_headers": extra,
        "payload_sha256": hashlib.sha256(body).hexdigest(),
        "x_amz_date": req.headers["X-Amz-Date"],
        "canonical_request": canonical,
        "string_to_sign": sts,
        "authorization": req.headers["Authorization"],
    })
print(json.dumps(out, indent=2))
