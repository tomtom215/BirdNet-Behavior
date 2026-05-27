-- crates/birdnet-db/migrations/009_accounts.sql
-- O-15 · Accounts, sessions, and audit log.
--
-- Builds on O-14 (cookie sessions). Day-zero shape: a single `admin` row
-- seeded from the existing single-admin password hash so behaviour is
-- unchanged until the operator visits /admin/accounts and adds a viewer
-- (or rotates the admin password).
--
-- Roll back with `010_accounts_down.sql` — drops the three tables.

CREATE TABLE users (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    username      TEXT NOT NULL UNIQUE,
    pwd_argon2    TEXT NOT NULL,
    role          TEXT NOT NULL
                  CHECK (role IN ('admin','viewer')),
    label         TEXT,                               -- friendly display name
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    disabled_at   TEXT
);

CREATE TABLE sessions (
    id           TEXT PRIMARY KEY,                    -- 26-char base32 of 128 random bits
    user_id      INTEGER NOT NULL
                 REFERENCES users(id) ON DELETE CASCADE,
    issued_at    TEXT NOT NULL DEFAULT (datetime('now')),
    last_seen    TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at   TEXT NOT NULL,
    user_agent   TEXT,                                -- truncated UA for display only
    ip_hash      TEXT                                 -- HMAC(ip, secret)[..16]; never raw IP
);
CREATE INDEX sessions_user_expires ON sessions (user_id, expires_at);
CREATE INDEX sessions_expires      ON sessions (expires_at);

CREATE TABLE audit_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    at           TEXT NOT NULL DEFAULT (datetime('now')),
    user_id      INTEGER REFERENCES users(id),
    action       TEXT NOT NULL,                       -- 'settings.update', 'audio.add', …
    target       TEXT,                                -- descriptor ("rule:nightjar-evenings")
    metadata     TEXT                                 -- JSON; never includes secret values
);
CREATE INDEX audit_log_at      ON audit_log (at DESC);
CREATE INDEX audit_log_action  ON audit_log (action, at DESC);

-- Seed the existing single admin so day-zero is identical to today.
-- The hash is copied from `settings.admin_password_hash` if present, else
-- the row is created with a sentinel that the auth middleware treats as
-- "no admin configured" (matching the current `AuthConfig::new` behaviour
-- when password is empty).
INSERT INTO users (username, pwd_argon2, role, label)
SELECT 'admin',
       COALESCE(value, ''),
       'admin',
       'Administrator'
  FROM settings
 WHERE key = 'admin_password_hash'
 LIMIT 1;

-- If no `admin_password_hash` row exists, still create the admin row so
-- foreign-key constraints in audit_log don't fail on first boot.
INSERT INTO users (username, pwd_argon2, role, label)
SELECT 'admin', '', 'admin', 'Administrator'
 WHERE NOT EXISTS (SELECT 1 FROM users WHERE username = 'admin');
