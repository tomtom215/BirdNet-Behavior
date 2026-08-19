#!/bin/sh
# Treat a blank BIRDNET_* environment variable as unset.
#
# Sourced by `docker/entrypoint.sh` before it reads any setting, and by
# `scripts/check-compose-startup.sh` so the gate exercises this code rather
# than a copy of it.
#
# Why
# ---
# `docker-compose.yml` interpolates optional settings as `${VAR:-}`, so every
# one of them lands in the container environment as an empty string whether or
# not the operator set it, and `.env.example` — which the docs tell you to
# `cp` to `.env` — ships many keys with an empty value for documentation.
#
# clap does not treat an empty environment variable as absent: it reads it as a
# supplied value. So `BIRDNET_LATITUDE=` is not "no latitude", it is "the
# latitude is the empty string", which fails to parse and exits 2 during
# argument parsing — before any of the daemon's own error handling runs. Four
# such variables were enough to stop `docker compose up` starting at all, and
# with `restart: unless-stopped` that is a loop rather than a failure anyone
# sees a cause for.
#
# `std::env::remove_var` is `unsafe` in edition 2024 and this workspace sets
# `unsafe_code = "forbid"`, so the binary cannot scrub its own environment.
# The image is where the blanks are manufactured, so the image is where they
# are removed.
#
# Blank means "unset" for every BIRDNET_* variable except the ones below, where
# an explicitly empty value is a documented setting in its own right.

# BIRDNET_IMAGE_CACHE_DIR: an explicitly empty value is the documented
# air-gapped opt-out — "no Wikipedia fetches" — and `src/cli.rs` carries a
# custom `OsStringValueParser` and a test specifically so it survives clap's
# stock PathBuf parser, which rejects empty values. Stripping it would silently
# re-enable image fetching on a station configured not to make them.
BNB_BLANK_IS_MEANINGFUL="BIRDNET_IMAGE_CACHE_DIR"

# Unset every blank (empty or whitespace-only) BIRDNET_* variable, except those
# named above. Reports what it removed, so an operator reading the container log
# can see why a setting they thought they made had no effect.
strip_blank_birdnet_env() {
    # `env` is the portable way to enumerate names; a value containing a newline
    # would confuse it, but no BIRDNET_* setting is multi-line.
    _bnb_blank=$(env | sed -n 's/^\(BIRDNET_[A-Za-z0-9_]*\)=[[:space:]]*$/\1/p')
    _bnb_stripped=""
    for _bnb_name in $_bnb_blank; do
        case " ${BNB_BLANK_IS_MEANINGFUL} " in
            *" ${_bnb_name} "*) continue ;;
        esac
        unset "${_bnb_name}"
        _bnb_stripped="${_bnb_stripped} ${_bnb_name}"
    done
    if [ -n "${_bnb_stripped}" ]; then
        echo "[entrypoint] ignoring blank settings (treated as unset):${_bnb_stripped}" >&2
    fi
    unset _bnb_blank _bnb_name _bnb_stripped
}
