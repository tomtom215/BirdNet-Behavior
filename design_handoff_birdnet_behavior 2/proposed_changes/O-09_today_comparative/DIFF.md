# O-09 · Today — comparative phrasing + structure

<!-- BNB:STATUS-HEADER -->
> **Risk:** none · **Priority:** 1 · **Status:** ready for review
> Acceptance: [VERIFY.md § O-09](../VERIFY.md#o-09--today--comparative-phrase) · Rollback: [ROLLBACK.md § O-09](../ROLLBACK.md#o-09--today--comparative-phrase)
<!-- BNB:STATUS-HEADER -->


## What

- Replaces `crates/birdnet-web/templates/today.html` with a richer header (comparative phrase + filter row + sticky search) while reusing every existing htmx partial unchanged.
- Adds one new Rust partial: `GET /pages/today-phrase` returning a one-line "A *loud* morning." headline computed against the user's own 30-day baseline.

## Files

| Action | Path |
|---|---|
| Replace | `crates/birdnet-web/templates/today.html` |
| Add     | `crates/birdnet-web/src/routes/pages/today_phrase.rs` |
| Patch   | `crates/birdnet-web/src/routes/pages/today.rs` — register route |

### Wiring the route

In `today.rs::router()` add:

```rust
.route("/pages/today-phrase", get(today_phrase_partial))
```

and import it:

```rust
mod today_phrase;
use today_phrase::today_phrase_partial;
```

That's the whole patch.

## Why

Production says *"You're listening."* on every load — same headline on a 4-detection day as a 4,000-detection day. The mockup hinted at emotional phrasing tied to data. This computes the percentile of today's count within the user's last 30 days and picks one of six tiers (quiet · calm · steady · busy · loud · record). Single SQL query, no schema changes, no new tables, no external services.

## Behavior matrix

| Today's percentile | Headline verb | Color var |
|---|---|---|
| ≤ 10% | *quiet* | `--fg-3` |
| 10–35% | *calm* | `--fg-2` |
| 35–65% | *steady* | `--fg` |
| 65–85% | *busy* | `--moss-ink` |
| 85–97% | *loud* | `--moss-ink` |
| > 97% | *record* | `--rare` |

Time-of-day suffix (`morning` / `midday` / `evening` / `night`) is derived from server clock — independent of percentile.

## Tests

`today_phrase.rs` ships with unit tests for percentile math and tier boundaries. Run with:

```sh
cargo test -p birdnet-web today_phrase
```

## Optional extras

- Cache the partial in process memory for 30s — current poll cadence in the template is `every 5m` so this isn't urgent, but cheap to add.
- Localize the verb table. Currently English-only; would need a small `i18n.toml` if you expand the language packs to UI strings (not just BirdNET label packs).

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* **Apply order:** shipped in the combined PR — see [HANDOFF.md](../HANDOFF.md#what-ships-in-this-pr) for the full file list.
* **Acceptance criteria:** [VERIFY.md § O-09](../VERIFY.md#o-09--today--comparative-phrase).
* **Rollback:** [ROLLBACK.md § O-09](../ROLLBACK.md#o-09--today--comparative-phrase).
* **Preview:** open [`INDEX.html`](../INDEX.html#O-09) for the rendered screen.
<!-- BNB:CROSSREF-FOOTER -->
