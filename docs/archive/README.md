# Archive: working records of earlier sessions

**These are not current documentation.** Each file is a dated working record: an
audit or a release plan written by one session against one commit, kept because
the reasoning in it is still the best account of why something was done. A
finding in one of them may since have been fixed, moved, or superseded, and
`file:line` evidence in them is as of the commit they name. Nothing here is kept
up to date and nothing in the manual links to it as instructions.

If you want to know what the software does now, read the manual
([`../book/`](../book/)), and for what is open now,
[`../HANDOFF.md`](../HANDOFF.md) and [`../FEATURE_GAP_ANALYSIS.md`](../FEATURE_GAP_ANALYSIS.md).

## What is here, and what it describes

| File | Written | Against | Status in its own words |
|---|---|---|---|
| [`PRODUCTION_AUDIT.md`](PRODUCTION_AUDIT.md) | 2026-08-17 | `main` at `f025cbd`: `v0.14.0` plus five commits. Re-checked at `ee795ed` (`v0.15.0`) on 2026-09-04; line numbers re-verified at `dd10fe7` on 2026-09-07 | Superseded by the field-readiness and post-0.14.0 audits; its own header says fifteen of its sixteen numbered findings are fixed |
| [`FIELD_READINESS_AUDIT.md`](FIELD_READINESS_AUDIT.md) | 2026-08-19 | the tree after `v0.14.0` | A second pass; its findings are left as first written and §8 records what moved |
| [`POST_0140_AUDIT.md`](POST_0140_AUDIT.md) | 2026-08-21; §4 added 2026-08-24 | `9615b9c`, the shipped `v0.14.0` line. Re-checked 2026-09-04 at `ee795ed` (`v0.15.0`) | "All twenty-five are fixed"; its §2 direct answers were written about `0.14.0` and have aged |
| [`ENCLOSURE_READINESS_AUDIT.md`](ENCLOSURE_READINESS_AUDIT.md) | 2026-08-26 | `7c6bf77`: `v0.14.0` plus the two audit branches merged | A third pass made by running the software rather than reading it |
| [`RELEASE_READINESS.md`](RELEASE_READINESS.md) | 2026-06-03 | an integration branch (`claude/gallant-feynman-bJs95`) that no longer exists | Marked superseded and "historical record only" by its own first lines |

## Related records that are still in `docs/`

Two more records of the same kind sit in [`..`](../) and not here, and so does the
long audit that is still updated in place:

- [`../RELEASE_PLAN.md`](../RELEASE_PLAN.md) (audited 2026-08-08) is the record of
  the `v0.10.x` cycle, which shipped as `0.11.0`; and
  [`../RELEASE_PUNCHLIST.md`](../RELEASE_PUNCHLIST.md) (2026-05-29) is older still.
  Both are marked superseded in their own first lines. They were **not** moved
  because a source comment outside this documentation tree names each path
  (`scripts/hardware-test.sh`, `crates/birdnet-web/tests/inline_style_guard.rs`,
  `.claude/hooks/session-start.sh`); move them when those comments are changed.
- [`../UNATTENDED_DEPLOYMENT_AUDIT.md`](../UNATTENDED_DEPLOYMENT_AUDIT.md) (343 KB;
  "what a year alone in a field would find", 2026-09-03, reconciled 2026-09-04 and
  2026-09-07) stays because Rust comments cite its row identifiers by path
  (`crates/birdnet-web/src/metrics.rs`, `crates/birdnet-web/src/security.rs`). Its
  rows are updated in place, which makes it the one working record that is still
  being maintained.

## Where these files used to be

Until this directory existed the five files above were `docs/<NAME>.md`. Released
changelog entries (`0.12.0` and `0.15.0`) and the files themselves still name them by
that old path, which is what the path was at the tag they describe.
