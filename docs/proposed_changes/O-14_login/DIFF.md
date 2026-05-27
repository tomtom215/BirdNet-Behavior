# O-14 · Branded login + cookie-session migration

<!-- BNB:STATUS-HEADER -->
> **Risk:** medium — replaces the auth boundary's user surface (the WWW-Authenticate dialog) with a real login page. The wire-level auth method changes from HTTP Basic to a signed cookie session. Single admin, single secret, single station — same trust model. · **Priority:** 2 · **Status:** RFC + drop-in templates
> Acceptance: VERIFY.md § O-14 · Rollback: ROLLBACK.md § O-14
<!-- BNB:STATUS-HEADER -->


## What

`auth.rs` today gates `/admin/*` behind HTTP Basic Auth. That has three end-user consequences worth fixing:

1. **No branding.** The credential prompt is the browser's native dialog. Different typeface, different palette, no station name, no helpful copy.
2. **No error states.** Wrong password → another native dialog, no helpful "Caps lock is on" hint, no rate-limit feedback.
3. **No "sign out".** Once Basic Auth credentials are cached, the only way out is to close the browser or clear site data. The dashboard remembers nothing about *who is logged in* because Basic Auth sends credentials on every request.

This change replaces Basic Auth with a thin session cookie + a real login page at `/login`. The credential **store** is unchanged — single admin, password hashed with `argon2id`, same constant-time check. What changes is the wire shape: instead of the browser sending `Authorization: Basic …` every request, the server issues a signed cookie that carries the session id + an HMAC-validated expiry.

This DIFF is an **RFC plus drop-in templates**. The session-cookie model needs an engineering sign-off; the login template ships ready-to-paste either way (the `<form method="post" action="/login">` HTML is the same regardless of which session mechanism backs it).

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/templates/login.html` — the page body |
| Add | `crates/birdnet-web/src/routes/auth_pages.rs` — `GET /login` + `POST /login` + `POST /logout` |
| Replace | `crates/birdnet-web/src/auth.rs` — Basic Auth path → cookie session path (see RFC below) |
| Patch | `crates/birdnet-web/src/server.rs` — register `auth_pages::router()` on the public side; auth middleware reads cookies instead of `Authorization:` |
| Patch | `crates/birdnet-web/templates/layout.html` — add a "Sign out" link in the topnav-right ribbon, visible only when authenticated |
| Patch | `.env.example` — note the new `BNB_SESSION_SECRET` env var |

## RFC — session-cookie shape

The minimum that resolves all three issues above without introducing a database table or a session-server crate:

- **Cookie name:** `bnb-session`
- **Cookie value:** `v1.{expires-ms}.{hmac-sha256(expires-ms, secret)}`
- **Attributes:** `HttpOnly; SameSite=Lax; Path=/; Max-Age={ttl}`. `Secure` set when `BNB_PUBLIC_URL` starts with `https://`.
- **Secret source:** `BNB_SESSION_SECRET` env var. If unset, derive deterministically from the existing admin password hash (one-time `BLAKE3(hashed_password || "session-v1")`) so an admin doesn't have to set a second secret on day one. Rotating the password rotates the secret, which signs out every existing session — that's the right semantics.
- **TTL:** 14 days, rolling (extended on every authed request). Configurable via `BNB_SESSION_TTL_DAYS`.
- **Validation:** middleware decodes the cookie, checks `expires_ms > now`, recomputes the HMAC, constant-time compares. On any failure → strip cookie + redirect to `/login?next={original_path}`.
- **CSRF:** every state-changing admin form gets a hidden `_csrf` field whose value is `HMAC(session_id, "csrf")`. The middleware verifies it on `POST`/`PUT`/`PATCH`/`DELETE`. (The current `hx-confirm` flow is unrelated and stays.)
- **Excluded paths:** the existing `is_excluded` set keeps working — `/api/v2/health`, `/api/v2/ws/detections`, `/static/*`, `/r/<token>` (public share links), `/feeds/*` (RSS / iCal), `/login`. Everything else under `/admin/*` requires a valid cookie.

Zero new crates — `hmac`, `sha2`, `subtle`, `base64` are already in the dep tree (used elsewhere in `birdnet-db` / `birdnet-integrations`).

## What the login page is

The drop-in `templates/login.html` ships:

- A centred ~400px card with the BirdNet wordmark and a circular "signal" brand mark.
- Eyebrow `BIRDNET BEHAVIOR`, serif `Sign in`, muted subtitle `Administer this station`.
- Real `<form method="post" action="/login">` with autocomplete, `username` / `password`, a "Remember me on this device" checkbox (extends TTL to 90 days), and a show/hide password toggle implemented in 30 lines of vanilla JS.
- A `data-error` slot that renders `Incorrect username or password.` when `?error=1` is in the URL — the POST handler 303-redirects to `/login?error=1&next=…` on failure.
- A muted footer link `← Back to dashboard` (public — `/`).
- The same top nav + footer chrome from `layout.html` so the page reads as part of the app, not a separate auth product.
- A `next` hidden field on the form preserved across error redirects.
- A small inline rate-limit indicator pulled from the existing `rate_limit.rs` — after 5 failed attempts in a minute, the error message changes to *"Too many attempts. Try again in 30s."* and the form button is disabled.

## What the logout flow is

- A `<form method="post" action="/logout">` mounted in the topnav-right of `layout.html`, visible only when the request carried a valid cookie.
- The button reads `Sign out` (lowercase verb, sentence-case noun — same tone as the rest of the chrome).
- Server: clear cookie (`Set-Cookie: bnb-session=; Max-Age=0`), redirect to `/` (the dashboard remains viewable without auth — only `/admin/*` is gated).

## Migration & rollback

- **Migration path.** First boot after upgrade: the existing password hash is unchanged, the session secret derives from it, no operator action required. Any open `/admin/*` tab sees a 302 to `/login` on next request; logging in once issues the cookie.
- **Rollback.** Reverting `auth.rs` to the Basic Auth version is a single git revert; the `/login` page stays harmless (just renders an unstyled form whose POST 404s).

## What this is *not*

- Not a multi-user system. There's still one admin. O-15 (Accounts / Sessions) builds on this and adds the multi-user surface.
- Not a remote-auth integration. Not OAuth, not OIDC, not LDAP. The session is local to this station.
- Not a password-reset flow. If the operator loses the password they SSH in and run `bnb admin reset-password` — same as today.

## Risk

Medium because it changes the wire-level auth model. Low because the trust boundary is unchanged: one secret, one admin, constant-time compare. Tests in `auth.rs` are extended (not replaced) to exercise the cookie path.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* O-15 (Accounts / Sessions) is built on top of this cookie model — adds "active sessions", "revoke all but this one", and an optional second admin role.
* O-17 (confirm modal) and O-18 (toast) already work without auth; this PR doesn't touch them.
<!-- BNB:CROSSREF-FOOTER -->
