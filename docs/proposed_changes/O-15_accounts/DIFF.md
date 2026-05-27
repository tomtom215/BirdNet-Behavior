# O-15 · Accounts & sessions surface

<!-- BNB:STATUS-HEADER -->
> **Risk:** medium · **Priority:** 4 — depends on O-14 landing first · **Status:** RFC + drop-in templates
> Acceptance: VERIFY.md § O-15 · Rollback: ROLLBACK.md § O-15
<!-- BNB:STATUS-HEADER -->


## What

Once O-14 replaces Basic Auth with cookie sessions, three small features become possible — and the dashboard's single-admin-on-an-honour-system needs them once more than one person uses the station:

1. **Active sessions list** — every cookie outstanding, with device label, IP, last seen, "Sign out everywhere else".
2. **Additional users** — a second admin, plus a read-only *Viewer* role for households where one person tends the Pi and others just want the dashboard without seeing the danger zone.
3. **Audit log** — a short trailing list of admin-side mutations ("Updated species filter · admin@station · 12 min ago"), shown on `/admin/overview` and persisted as a SQLite table.

Each ships as a card on a new `/admin/accounts` page (under the existing admin sub-nav) with the design vocabulary used by the rest of admin (sticky sidebar / main panel, no rail). Nothing on the public side of the app changes.

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/templates/admin_accounts.html` — body for `/admin/accounts` (admin shell wraps it) |
| Add | `crates/birdnet-web/src/routes/admin/accounts.rs` — page + partials + POST/DELETE handlers |
| Add | `crates/birdnet-db/migrations/009_accounts.sql` — `users`, `sessions`, `audit_log` tables |
| Add | `crates/birdnet-db/src/accounts.rs` — `UserStore`, `SessionStore`, `AuditLog` traits |
| Patch | `crates/birdnet-web/src/routes/admin/mod.rs` — add `pub mod accounts;` + nav entry |
| Patch | `crates/birdnet-web/src/auth.rs` — `current_user(req) -> Option<User>` helper for downstream handlers + role gate |
| Append | `crates/birdnet-web/static/css/app.css` — see `css/app.css.append` |

## Schema

```sql
CREATE TABLE users (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    username     TEXT NOT NULL UNIQUE,
    pwd_argon2   TEXT NOT NULL,         -- argon2id hash; identical algorithm to the current admin
    role         TEXT NOT NULL CHECK (role IN ('admin','viewer')),
    label        TEXT,                  -- friendly name shown on the audit log
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    disabled_at  TEXT
);

CREATE TABLE sessions (
    id           TEXT PRIMARY KEY,      -- random 128-bit, base32-encoded
    user_id      INTEGER NOT NULL REFERENCES users(id),
    issued_at    TEXT NOT NULL DEFAULT (datetime('now')),
    last_seen    TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at   TEXT NOT NULL,
    user_agent   TEXT,
    ip_hash      TEXT                   -- first 8 bytes of HMAC(ip, secret) — never raw IP
);
CREATE INDEX sessions_user ON sessions (user_id, expires_at);

CREATE TABLE audit_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    at           TEXT NOT NULL DEFAULT (datetime('now')),
    user_id      INTEGER REFERENCES users(id),
    action       TEXT NOT NULL,         -- 'settings.update', 'audio.add', 'rule.toggle', …
    target       TEXT,                  -- arbitrary descriptor ("rule:nightjar-evenings")
    metadata     TEXT                   -- optional JSON blob (never includes raw values)
);
CREATE INDEX audit_log_at ON audit_log (at DESC);
```

Seed migration: a single `admin` row is created from the existing password hash, so day-zero behaviour is unchanged for everybody.

## Sessions card — wireframe

```
┌─────────────────────────────────────────────────────────────┐
│ ACTIVE SESSIONS                                              │
│                                                              │
│ 🔘 This device                                              │
│    macOS · Safari 17 · last seen just now    ⌗ #abcd      │
│                                                              │
│   Laptop                                                     │
│    macOS · Firefox 124 · last seen 4 days ago   Sign out → │
│                                                              │
│   Tablet                                                     │
│    iPadOS · Safari · last seen 18 days ago     Sign out → │
│                                                              │
│   ──────────────────────────────────────────                │
│    [ Sign out of every other device ]                       │
└─────────────────────────────────────────────────────────────┘
```

The "this device" row is marked but not given a Sign-out — signing yourself out is what the topnav `Sign out` link is for. The bulk button is the support escape hatch *"I think my password leaked"*. Endpoints: `DELETE /admin/accounts/sessions/{id}`, `POST /admin/accounts/sessions/revoke-others`.

## Users card — wireframe

```
┌─────────────────────────────────────────────────────────────┐
│ USERS                                                        │
│                                                              │
│  admin   ADMIN     created 2025-04-22       Reset password   │
│  jess    VIEWER    created 2026-02-14       Edit · Disable  │
│                                                              │
│   ──────────────────────────────────────────                │
│   [ Invite a viewer ]                                       │
└─────────────────────────────────────────────────────────────┘
```

"Invite a viewer" opens an inline `<details>` with a username + password form (no email — this is single-station, single-LAN, no SMTP path). Adding a second admin is *also possible* but the affordance is in a secondary "Promote to admin" action on a viewer row, never a default. The `admin` user can't be deleted; can only have its password rotated.

Role gate in `auth.rs`:

```rust
pub fn require_admin(req: &Request) -> Result<User, Response> {
    let u = current_user(req).ok_or_else(redirect_to_login)?;
    if u.role != Role::Admin { return Err(forbidden_page()) }
    Ok(u)
}
```

Every existing `/admin/*` handler that mutates state calls `require_admin`; read-only `/admin/*` handlers call `require_user`. `viewer` users can see `/admin/overview`, `/admin/quality`, `/admin/notifications` (history), `/admin/system`, and the audit log — they cannot reach `/admin/settings`, `/admin/audio`, `/admin/rules`, `/admin/migration`, or `/admin/system_controls/*`.

## Audit log card — wireframe

```
┌─────────────────────────────────────────────────────────────┐
│ RECENT CHANGES                                               │
│                                                              │
│  admin       updated audio source       2m ago              │
│              src_usb_1 · gain +6 → +9 dB                    │
│  admin       toggled rule               1h ago              │
│              "Owls after 22:00" → enabled                   │
│  admin       restarted service          2h ago              │
│  jess        signed in                  4h ago              │
│  admin       added user                 yesterday           │
│              jess (viewer)                                  │
│                                                              │
│  Full log →                                                  │
└─────────────────────────────────────────────────────────────┘
```

The full log lives at `/admin/audit` and is a single column of rows + a date-range picker. Retention: 180 days (configurable).

The **`AuditLog::record(action, target, metadata)`** helper is called from every mutating endpoint via a thin wrapper in `routes/admin/mod.rs`. The metadata column never stores secret values (passwords, API keys, RTSP credentials are filtered before persistence — same rule as `metrics.rs`).

## Risk

Medium. Schema is additive; auth path becomes role-aware. Existing single-admin deployments see one extra row in the users table and no behavioural difference. Reverting drops the new tables and rolls auth back to single-admin gating.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* Blocked on O-14 (cookie sessions).
* Audit-log entries link out to the relevant settings page; uses O-19 (cmdk) `Settings` index to keep the deep-links stable.
* Sessions card uses O-17 modal for "Sign out of every other device" confirmation.
<!-- BNB:CROSSREF-FOOTER -->
