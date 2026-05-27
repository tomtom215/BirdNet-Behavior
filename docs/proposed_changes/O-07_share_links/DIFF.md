# O-07 · Shareable rare-bird permalinks

<!-- BNB:STATUS-HEADER -->
> **Risk:** medium · **Priority:** 4 · **Status:** ready for review
> Acceptance: [VERIFY.md § O-07](../VERIFY.md#o-07--rare-bird-permalinks) · Rollback: [ROLLBACK.md § O-07](../ROLLBACK.md#o-07--rare-bird-permalinks)
<!-- BNB:STATUS-HEADER -->


## What

Public, read-only HTML share pages for individual detections at `/r/<token>`. No auth, no nav, no surface to the admin — just a clean editorial card a station owner can paste into a group chat or post to a forum.

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/templates/share_rare.html` |
| Add | `crates/birdnet-web/src/routes/share.rs` |
| Patch | `crates/birdnet-web/src/routes/mod.rs` — `pub mod share;` and `.merge(share::router())` |
| Optional patch | quarantine + detection_detail templates — add a "Share clip" button that calls the helper below |
| Optional env | `BNB_SHARE_SECRET=…` (32+ random bytes) — without it tokens are random per restart, which means restarts invalidate every outstanding link. |

## How tokens work

```
token = base64url("<id>:<expiry_epoch>." || HMAC-SHA256(secret, "<id>:<expiry_epoch>")[..16])
```

- Anyone with a token can read **only** that detection's HTML, wav, and spectrogram. They cannot enumerate other IDs because the HMAC anchors the id to the secret.
- Tokens are 30-day TTL by default. Re-share generates a fresh token.
- HMAC truncated to 128 bits — overkill for this threat model, still 2^64 work to forge.
- `constant_time_eq` for the verify step to avoid timing oracles.

## Surfacing the share button

In quarantine review and `/detection/<id>`:

```rust
let token = share::encode_share_token(det.id, now + 30 * 86400);
let url = format!("/r/{token}");
// Render a "Share clip →" button with hx-get="…&copy=…" or plain JS clipboard.
```

## The two stubs left for the implementer

1. `share_audio` and `share_spectrogram` need to call into the existing `recordings.rs` / `spectrogram.rs` modules to stream bytes. I left them returning 404 with a `_ = (state, id);` so the file compiles.
2. `hmac_sha256` is a placeholder. The workspace already depends on `axum` and a crypto layer somewhere; swap for `hmac::Hmac::<Sha256>::new_from_slice(...)` — ~10 lines. The token shape doesn't change.

## OpenGraph

The template includes `og:title / og:description / og:image / twitter:card` so when the link is pasted into Slack / iMessage / Mastodon, it renders as a card with the spectrogram as the preview image. This is the killer feature — the link sells itself.

## Risk

Low. New routes; no schema changes. The default per-restart secret means a server restart invalidates all outstanding links — set `BNB_SHARE_SECRET` in production to fix that.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* **Apply order:** shipped in the combined PR — see [HANDOFF.md](../HANDOFF.md#what-ships-in-this-pr) for the full file list.
* **Acceptance criteria:** [VERIFY.md § O-07](../VERIFY.md#o-07--rare-bird-permalinks).
* **Rollback:** [ROLLBACK.md § O-07](../ROLLBACK.md#o-07--rare-bird-permalinks).
* **Preview:** open [`INDEX.html`](../INDEX.html#O-07) for the rendered screen.
<!-- BNB:CROSSREF-FOOTER -->
