# Reviewing Detections

BirdNet-Behavior classifies every clip automatically, but you are the final
arbiter. The **detection-review** queue lets you record a verdict —
*confirmed* or *rejected* — on individual detections, building a clean,
human-checked record over time.

![The detection-review triage queue](../images/detection-reviews.png)

## Reviews vs. quarantine

These are two different quality-control surfaces; it helps to keep them straight:

| | [Quarantine](../admin/settings.md) | Detection reviews |
|---|---|---|
| **What it holds** | Detections that *failed* a stricter per-species threshold | Detections already admitted to the log |
| **Effect** | Gates rows *out* of `detections` until you approve them | A non-destructive annotation; nothing is moved or deleted |
| **Verdict** | Approve / Reject / Delete | Confirm / Reject (reversible) |
| **Use it to** | Vet uncertain rare birds before they count | Audit the ID quality of detections that already count |

![The quarantine queue, where detections that failed a stricter per-species threshold wait for your approve / reject / delete verdict](../images/quarantine.png)

A *rejected* review flags a likely misidentification for your own records — it
does **not** remove the detection. Quarantine is the tool for keeping a dubious
record out of the log entirely.

## The triage queue (`/detection-reviews`)

The queue lists recent detections that have **no verdict yet**, newest first,
each with its species, time, and confidence. Two buttons record a verdict:

- **✓ Confirm** — the identification looks right.
- **✗ Reject** — likely a misidentification.

A running tally at the top shows how many detections you have confirmed,
rejected, and have left to review. Recorded verdicts move to the **Recent
verdicts** list, where an **Undo** button clears a verdict and returns the
detection to the queue. Re-reviewing a detection simply replaces its previous
verdict — there is never more than one verdict per `(date, time, species)`.

## Reviewing from a detection

You don't have to work the queue to leave a verdict. Every
[detection-detail page](./sharing.md) carries a **Review this detection**
widget with the same Confirm / Reject buttons and a badge showing the current
verdict, so you can judge a clip the moment you're looking at it — spectrogram,
audio and all.

## Comments: why, not just what

A verdict says *what* the station decided. Six months later the question is
*why* — and "call length says Downy, but the spectrogram is Hairy" is the
sentence that makes a record defensible to somebody who was not there.

Under the review widget, every detection-detail page carries a **Comments**
thread. Write a note, it appears with your username and the time, and it stays
that way: a comment is **never edited**. The database refuses it — there is a
trigger on the table that aborts any attempt to rewrite a comment's text,
author or timestamp — so what you read is what was written.

Two things follow from that, and both are deliberate:

- **Many comments per detection.** Two observers disagreeing is the point. This
  is the difference from the review widget's own notes field, which is keyed on
  the detection and *replaces* whatever was there: a second reviewer's note
  silently erased the first one's reasoning, and neither of them was named.
- **A comment can be withdrawn, not rewritten.** *Remove* deletes it outright —
  which is what a note containing a typo, or a neighbour's name, needs. The
  audit log records the removal, its author and its id, and never the text: a
  comment deleted because of what it said must not survive in the log that
  recorded its deletion.

Comments are up to 2 000 characters, and writing one needs an admin sign-in —
the same gate as confirming or rejecting. Anyone who can see the detection can
read the thread.

Scripts can join in through the API: `GET`, `POST` and
`POST …/delete` on `/api/v2/detections/comments` (see the
[API reference](../reference/api.md)). Those comments are attributed to `api`
rather than to a name the caller supplies — a bearer token is not a person, and
a name the caller chose is not attribution. A script with something to say
about who is speaking says it in the comment.

## Sharing a detection for a second opinion

Not sure about a call? Both the detection-detail page and every row in the
[quarantine queue](../admin/settings.md) have a **Share** button that copies a
public [`/r/<token>` link](./sharing.md). The share page
resolves quarantined rows too, so you can send a rare bird that hasn't been
admitted to the log to another birder for a second opinion before you decide.
