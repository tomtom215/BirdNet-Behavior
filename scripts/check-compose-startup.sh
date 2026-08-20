#!/usr/bin/env bash
# Gate: the documented Docker quickstart must actually start the daemon.
#
# Why this exists
# ---------------
# `docker-compose.yml` interpolates optional settings as `${VAR:-}`, so
# `docker compose config` materialises them into the container environment as
# empty strings whether or not the operator set them. clap reads an empty
# environment variable as a *supplied* value, so `BIRDNET_LATITUDE=` is not
# "no latitude" — it is "the latitude is the empty string", which fails to
# parse and takes the process down with exit 2 before anything starts.
#
# Four such variables were enough to make `docker compose up` — the path
# `docs/book/getting-started/docker.md` documents — unable to start at all,
# in a loop, because `restart: unless-stopped` retries forever. Nothing caught
# it: the only container check in `.github/workflows/docker.yml` runs
# `--verify-extension` with `--entrypoint` bypassed and no environment at all,
# and the 2 269-test Rust suite never sees an environment variable.
#
# What it checks
# --------------
# Resolves the real container environment with `docker compose config` (offline,
# no daemon needed), applies `docker/entrypoint.sh`'s blank-stripping, and runs
# the real binary under it. The daemon must still be up after the grace period.
#
# Usage:  scripts/check-compose-startup.sh [path-to-binary]
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT" || exit 1

BIN="${1:-target/debug/birdnet-behavior}"
GRACE_SECS="${GRACE_SECS:-20}"
PORT="${PORT:-18787}"

if [ ! -x "$BIN" ]; then
  echo "check-compose-startup: no binary at $BIN — build it first" >&2
  exit 1
fi
if ! docker compose version >/dev/null 2>&1; then
  echo "check-compose-startup: 'docker compose' not available — skipping" >&2
  exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# The environment `docker compose up` would put in the container. YAML scalars
# come back quoted, so unquote them; `KEY: ""` becomes `KEY=`.
docker compose config 2>/dev/null \
  | awk '/^    environment:/{f=1;next} /^    [a-z]/{f=0} f' \
  | sed 's/^ *//; s/: /=/' | sed 's/="\(.*\)"$/=\1/' > "$WORK/env.txt"

if [ ! -s "$WORK/env.txt" ]; then
  echo "check-compose-startup: could not resolve the compose environment" >&2
  exit 1
fi

blank=$(grep -E '^BIRDNET_[A-Z0-9_]+=$' "$WORK/env.txt" || true)
total=$(wc -l < "$WORK/env.txt")
echo "check-compose-startup: compose supplies $total variables"

# (1) No blank BIRDNET_* in the container's *configured* environment.
#
# `docker/entrypoint.sh` strips blanks too, but only for the daemon it execs.
# `docker compose exec birdnet birdnet-behavior --doctor` — the troubleshooting
# command the docs recommend — gets the configured environment instead, so a
# blank there is still a broken command even with the entrypoint backstop.
if [ -n "$blank" ]; then
  echo "check-compose-startup: FAILED — compose puts blank values in the container:" >&2
  echo "$blank" | sed 's/^/    /' >&2
  echo "  clap reads a blank environment variable as a supplied value; write these" >&2
  echo "  as comments and let env_file supply them from .env instead." >&2
  exit 1
fi

# (1b) `.env.example` — which step 1 of the quickstart tells you to `cp` to
# `.env` — must ship no key with a blank value either. `docker compose config`
# does not expand `env_file`, so this is a separate check, and it matters for
# the same reason: those values reach `docker compose exec` unfiltered.
env_blank=$(grep -nE '^BIRDNET_[A-Z0-9_]+=$' .env.example || true)
if [ -n "$env_blank" ]; then
  echo "check-compose-startup: FAILED — .env.example ships blank keys:" >&2
  echo "$env_blank" | sed 's/^/    /' >&2
  echo "  comment them out; a blank value is a supplied value, not an unset one." >&2
  exit 1
fi

# (2) The daemon actually starts under that environment, unscrubbed.
set -a
while IFS= read -r line; do
  [ -z "$line" ] && continue
  eval "export ${line%%=*}=\"\${line#*=}\""
done < "$WORK/env.txt"
set +a

# The image supplies these; without a model this run is --web-only.
unset BIRDNET_MODEL BIRDNET_LABELS BIRDNET_ANALYTICS_DB BIRDNET_WATCH_DIR BIRDNET_IMAGE_CACHE_DIR
printf 'DB_PATH=%s/birds.db\n' "$WORK" > "$WORK/birdnet.conf"

"$BIN" --config "$WORK/birdnet.conf" --listen "127.0.0.1:$PORT" --web-only \
  > "$WORK/out.log" 2>&1 &
pid=$!

up=0
for _ in $(seq 1 "$GRACE_SECS"); do
  if ! kill -0 "$pid" 2>/dev/null; then break; fi
  if curl -sf -o /dev/null "http://127.0.0.1:$PORT/api/v2/health" 2>/dev/null; then up=1; break; fi
  sleep 1
done

if [ "$up" = "1" ] && kill -0 "$pid" 2>/dev/null; then
  kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
  echo "check-compose-startup: OK — the daemon came up under the compose environment"
  exit 0
fi

wait "$pid" 2>/dev/null; rc=$?
echo "check-compose-startup: FAILED — the daemon did not come up (exit $rc)" >&2
echo "--- last lines ---" >&2
tail -20 "$WORK/out.log" >&2
exit 1
