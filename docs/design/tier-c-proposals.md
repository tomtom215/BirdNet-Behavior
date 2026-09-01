# Five proposals that need a decision before any code

These are the items from the [parity audit](../../CHANGELOG.md) that cannot be
started without committing to an architecture. Each is weeks of work and each
forecloses options once begun, so they are written down here rather than built.

Nothing in this file is implemented. Each section says what the thing is, what
it would cost, what it would rule out, and — where there is one — the cheaper
partial that captures most of the value.

The comparison throughout is [tphakala/birdnet-go](https://github.com/tphakala/birdnet-go),
which has shipped all five.

---

## 1. Multi-model support and a model gallery

### What it is

Today the station loads exactly one ONNX classifier from a path resolved at
startup. birdnet-go treats models as installable content: a catalogue with
checksums, in-app install and removal with progress, several models resident at
once, a model assigned per audio source, and cross-model consensus recorded per
detection.

### What it would cost here

The single-model assumption is not localised. It is baked into:

- `BirdNetModel` owning one `ort::Session` and one `LabelSet`, with
  `infer_sample_rate()` deciding the *pipeline's* resample target — two models
  wanting 48 kHz and 32 kHz cannot share one `PipelineConfig` as it stands.
- `SpeciesFilter`, which aligns the geomodel against *the* classifier's labels.
  With two classifiers there are two vocabularies to align, and the guard added
  in `a8e6268` would need a per-model answer.
- The `detections` schema, which has no model column. Cross-model consensus
  needs one row per contribution or a join table, and every analytics query in
  `birdnet-behavioral` and `birdnet-timeseries` assumes one row per detection.

### The decision

Consensus is the expensive half, not the gallery. Running two models and
reporting both is additive; **agreeing** them means deciding what a detection
*is* — and every existing aggregate, life list and rare-species feed answers
against that definition.

### Cheaper partial

A model **switcher** (one model at a time, chosen in the UI, downloaded and
checksum-verified like the geomodel already is) needs no schema change and no
consensus semantics. It captures the "try Perch on this station" use case,
which is most of what operators actually ask for, and leaves the multi-model
question open.

**Recommendation:** build the switcher; treat consensus as a separate proposal
once there is evidence anyone wants two models resident at once.

---

## 2. MySQL as an alternative datastore

### What it is

birdnet-go supports SQLite or MySQL. The draw is not performance — it is
several stations writing to one database, which is how a multi-site deployment
gets one dashboard.

### What it would cost here

`rusqlite::Connection` is threaded through everything: `AppState::with_db`
hands out `&Connection`, every query in `birdnet-db` is written against it, and
the migration framework applies SQLite DDL. An abstraction over both engines is
a rewrite of the storage layer, not an addition to it.

The harder half is DuckDB. `birdnet-behavioral` and `birdnet-timeseries` attach
the SQLite file directly and query it in place. Against MySQL that route does
not exist, so behavioural analytics would need either a replication path or a
second implementation — and those analytics are this project's distinguishing
feature.

### The decision

MySQL is not really a storage question. It is a question about whether this
project wants to be a **multi-station** system, which also implies identity,
per-station filtering across every analytics query, and a migration story for
existing single-station databases.

### Cheaper partial

A **read-only export or replication sink** — push detections to a central
Postgres/MySQL/TimescaleDB on a schedule, keep SQLite authoritative locally.
Multi-site dashboards work; nothing about the local station changes; the
analytics keep their DuckDB path.

**Recommendation:** the sink. Adopting a second OLTP engine to get a feature
that is really about fleet management is the wrong trade.

---

## 3. OIDC / SSO

### What it is

Google, GitHub, Microsoft and generic OIDC login, with RP-initiated logout,
subnet bypass and trusted-proxy handling. We have local accounts (Admin /
Viewer), cookie sessions, and an optional `CADDY_PWD`.

### What it would cost here

Less than it looks. `birdnet-db::accounts` already models users, roles,
sessions and an audit log; `auth_middleware` already validates a cookie and
attaches a `RequestUser`. OIDC adds a provider config, the authorisation-code
exchange, JWKS fetching and cache, and an identity→user mapping. It does *not*
require rethinking authorisation.

The real costs are operational: a station on a home LAN has no public
redirect URI, so this only helps someone already running a reverse proxy with a
domain — and it adds an outbound dependency at login, which is exactly what
`--offline` exists to avoid.

### The decision

Who is this for? A single-operator garden station gains nothing. An
institution running twenty stations gains a lot, and is also the deployment
most likely to have the proxy and the domain already.

### Cheaper partial

**Header-based trusted-proxy auth** (`X-Forwarded-User` from an authenticating
proxy such as oauth2-proxy or Authelia). Perhaps forty lines: trust the header
only from configured proxy IPs, map the value to a local user. Institutions get
SSO through infrastructure they already run; we take on no OIDC surface, no
JWKS cache, and no login-time egress.

**Recommendation:** the header path, gated on a `trusted_proxies` allow-list.
Revisit real OIDC only if someone reports the proxy route is not enough.

---

## 4. UI localisation

### What it is

birdnet-go ships 17 UI locales with 4,147 translated keys, validated in CI. Our
chrome is English-only; we translate *species names* into 36 languages, which is
a different thing and is already done.

### What it would cost here

The blocker is not translation, it is that our HTML is `format!` string
literals spread across ~50 route modules. There is no template layer to
externalise strings from. Introducing one means touching every page.

Two credible shapes:

1. **Extract to a catalogue** — a `t!("key")` macro over a compile-time map,
   with a CI check that no user-visible literal escapes it. Mechanical, large,
   and it makes the render code noticeably harder to read — which matters,
   because that code's readability is currently one of its strengths.
2. **Adopt a template engine** (askama, minijinja) — cleaner long-term, and a
   rewrite of the entire web layer.

Either way, translations are then a continuing obligation: a locale at 60 %
coverage is worse than English for a user who hits the untranslated 40 % mid-task.

### The decision

Is this a *product* commitment? Shipping two locales and letting them rot is
worse than shipping none. birdnet-go can carry seventeen because it validates
them in CI and has contributors who maintain them.

### Cheaper partial

Localise the **dashboard and the detection list** only — the screens a
non-administrator actually reads — and leave `/admin` in English. Perhaps 200
keys rather than 4,000, a bounded translation obligation, and it covers the
household member who is not the operator.

**Recommendation:** the bounded subset, and only alongside a commitment to a CI
coverage gate. Otherwise nothing.

---

## 5. Settings hot-reload

### What it is

Settings apply live. Ours are layered onto the file config at startup and the
form says so: *"Changes apply on next restart."*

### What it would cost here

`helpers::settings_overlay` reads the settings table once and produces a
`Config` that everything downstream copies values out of. Hot-reload means the
consumers hold a handle rather than a copy, and each decides what changing
means *while running*:

| Setting | Reloading it means |
|---|---|
| `confidence_threshold`, `sf_thresh` | trivial — read per detection |
| `species_include` / `exclude` | already live (`SpeciesListsProvider`, ~30 s TTL) |
| `latitude` / `longitude` | invalidate the occurrence-filter cache, recompute the schedule |
| `segment_duration`, `alsa_device`, `rtsp_url` | restart the capture process for that source |
| `listen`, session secret | genuinely needs a restart |

So "hot-reload" is not one feature. It is a per-setting decision, and the
honest version of it is a table like the one above rather than a global switch.

### Cheaper partial

The pattern already exists and already works: `SpeciesListsProvider` is a
closure re-read on a short TTL. Extending it to the handful of settings in the
first two rows is small, safe, and covers the ones an operator changes while
watching the dashboard — which is the actual complaint behind the feature
request.

Everything below that line keeps saying "applies on next restart", and the
settings page should say which is which per field rather than one blanket
notice.

**Recommendation:** do the cheap rows and make the notice per-field. A general
reload mechanism buys little beyond that and risks a station reconfiguring its
capture pipeline underneath a running recording.

---

## Summary

| Proposal | Recommendation | Rough size |
|---|---|---|
| Multi-model | Build the **switcher**; defer consensus | switcher ~1 wk; consensus ~4 wk + schema |
| MySQL | Build a **replication sink** instead | sink ~1 wk; MySQL ~6 wk + analytics rework |
| OIDC | Build **trusted-proxy header auth** instead | header ~1 day; OIDC ~2 wk |
| UI i18n | **Bounded subset**, with a CI coverage gate, or nothing | subset ~1 wk + ongoing |
| Hot-reload | **Per-setting**, starting with the trivial rows | ~2 days for the cheap rows |

Four of the five have a partial that delivers most of the value for a fraction
of the cost and does not foreclose the full version. That is the argument for
writing this down instead of starting any of them.
