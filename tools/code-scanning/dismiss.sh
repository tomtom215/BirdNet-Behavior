#!/usr/bin/env bash
# Dismissals for the code-scanning alerts that no code change can (or should) clear.
#
# Reviewed 2026-10-21 against the owner's list of 128 open alerts and a local CodeQL run that
# reproduced the 117 CodeQL alerts exactly (same rule, file and line; bundle v2.27.2, the version
# github/codeql-action v4.39.0 pins). Every other CodeQL alert on that list is fixed in code on the
# branch and closes itself after the next analysis of the default branch.
#
# Needs a token with `security_events: write` (a maintainer's own `gh auth login`; the Claude Code
# session token has `read` at most). Prints the commands by default; set APPLY=1 to run them:
#
#     tools/code-scanning/dismiss.sh            # prints
#     APPLY=1 tools/code-scanning/dismiss.sh    # dismisses
#
# Alert numbers are GitHub's, as of 2026-10-21. A dismissal is tied to the code's fingerprint, not
# the line: moving a dismissed line does not reopen it, changing it can. Re-read the code before
# dismissing; the reason has to be true on the day, not on the day this file was written.
set -euo pipefail
REPO="tomtom215/BirdNet-Behavior"

dismiss() { # number, reason (false positive | won't fix | used in tests), comment
  local n=$1 reason=$2 comment=$3
  if [ "${APPLY:-}" = 1 ]; then
    gh api -X PATCH "repos/$REPO/code-scanning/alerts/$n" \
      -f state=dismissed -f "dismissed_reason=$reason" -f "dismissed_comment=$comment" >/dev/null
    echo "dismissed #$n ($reason)"
  else
    printf 'gh api -X PATCH repos/%s/code-scanning/alerts/%s -f state=dismissed -f %q -f %q\n' \
      "$REPO" "$n" "dismissed_reason=$reason" "dismissed_comment=$comment"
  fi
}

# --- rust/path-injection (11): CodeQL models axum's `State<AppState>` extractor as a remote source ---
CONFIGURED="False positive. The path is station configuration fixed at startup: db_path() from the config file, its sibling backups/ directory, or recording_dir(). It reaches this code through AppState, which CodeQL models as a remote source via axum's State extractor. No request value is part of it (the handlers take no path parameter). The one admin action that deletes under it (clear_extracted) lists the directory's own entries."
for n in 21 22 26 28; do dismiss "$n" "false positive" "$CONFIGURED"; done          # crates/birdnet-db/src/resilience.rs
for n in 25 29; do dismiss "$n" "false positive" "$CONFIGURED"; done                # routes/admin/backup_recovery.rs
for n in 30 31; do dismiss "$n" "false positive" "$CONFIGURED"; done                # routes/admin/system_controls/data.rs (admin-only 'clear extracted recordings')
dismiss 27 "false positive" "False positive. recording_stats(dir) is called with the configured recordings directory (station configuration fixed at startup), never with a request value; CodeQL's source is the State extractor."
dismiss 24 "false positive" "False positive. The path is recording_dir() joined with Path::new(file_name).file_name() of a clip name taken from a detection row. file_name() strips every directory component and returns None for '..', so the result cannot leave the recording directory. The clip is removed only when no other detection names it."
dismiss 23 "false positive" "False positive. The path is the image-cache directory (configured) joined with '{cache_key}.jpg'. The key is the species name lowercased with spaces and slashes mapped to '_' (cache_key in species_images/mod.rs), so no separator survives. Admin-only endpoint."

# --- rust/command-line-injection (1) ---
dismiss 56 "false positive" "False positive. df runs with a fixed argv '-Pk -- <path>' and no shell. '--' is part of DF_ARGS (guarded by the test df_is_invoked_with_posix_flags_only), so a path starting with '-' is a path. The path is a configured data, recordings or model directory (every disk_usage caller), never a form value."

# --- js/http-to-file-access (1) ---
dismiss 19 "used in tests" "Used in tests: the visual-QA sweep (tools/visual-qa/qa.mjs) writes report.json from pages of the localhost fixture server its caller started. The file is JSON, never executed, served or built into anything, and no code reads it back; CI only uploads it as an artifact."

# --- Scorecard TokenPermissions (3): job-level 'contents: write' that creating a release needs ---
RELEASE="Won't fix. This job creates or edits a GitHub release with the gh CLI, which needs contents: write; no narrower scope exists (there is no 'releases: write' permission), and the workflow-level token is read-only. Scorecard ignores a job-level contents: write only when the workflow uses a release tool it recognises (semantic-release, goreleaser, slsa-github-generator, ...); this repository publishes with gh, and adding a decoy step would game the check."
for n in 2 3 166; do dismiss "$n" "won't fix" "$RELEASE"; done  # publish-model.yml:75, release.yml:904, publish-model.yml:347

cat <<'EOF'

Not dismissals; these are the maintainer's to settle (no file change clears them):
  #1   Scorecard BranchProtection   Settings -> Rules: enable `protect-main` and add the required checks
                                    (.github/workflows/README.md lists them).
  #8   Scorecard CodeReview         needs a second human reviewer on merged changes.
  #9   Scorecard CIIBestPractices   register at https://www.bestpractices.dev/ and add the badge.
  #165 Scorecard SecurityPolicy     SECURITY.md needs a reporting link or address, and private
                                    vulnerability reporting enabled (Settings -> Code security).
Closed by changes on the branch (they close after the next default-branch analysis):
  #154 Vulnerabilities (certifi), #4 (docker.yml), #6 (mutation.yml), #5 (upgrade-e2e.sh), every
  rust/hard-coded-cryptographic-value, cleartext-logging, log-injection and non-https-url alert,
  and every js/xss-through-dom, js/insecure-randomness and js/missing-origin-check alert.
EOF
