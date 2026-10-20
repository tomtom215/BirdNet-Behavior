#!/usr/bin/env bash
# installer/catalogue-field.sh — read one field of one model from the Rust
# catalogue (crates/birdnet-integrations/src/model_catalog.rs).
#
#   installer/catalogue-field.sh ID FIELD [CATALOGUE.rs]
#
#   installer/catalogue-field.sh birdnet-v24 model_sha256
#   installer/catalogue-field.sh birdnet-v24-range-v2 model_url
#   installer/catalogue-field.sh birdnet-v24-range-v2 model_bytes
#
# Prints the field's string or integer (underscores removed from integers), NONE
# for `None`, and nothing when the entry or the field is absent. The exit status
# is 0 either way; the caller decides what absence means.
#
# Why this exists: the catalogue is the one place a model's digest, origin and
# size are written down, and two things outside Rust need to read them — the
# test that holds the V3.0 pins in the installer and the container equal to it
# (installer/test/model-pins-match-catalogue.sh), and the workflow that mirrors
# the V2.4 files into a release (.github/workflows/publish-model.yml). Reading
# the Rust beats copying the values out of it, which is the duplication this is
# here to prevent. Pure bash, GNU sed: it joins the string continuations
# (`"…\<newline>   …"` is one string, which is how the long URLs are written),
# looks only inside the CATALOG table (the unit tests below it quote the same
# digests, and a parser that could fall into them would pass on the wrong
# copy), and takes the first string or integer after `FIELD:`.
set -euo pipefail

ID="${1:?usage: catalogue-field.sh ID FIELD [FILE]}"
FIELD="${2:?usage: catalogue-field.sh ID FIELD [FILE]}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FILE="${3:-${HERE}/../crates/birdnet-integrations/src/model_catalog.rs}"
[ -f "${FILE}" ] || { echo "catalogue-field.sh: no catalogue at ${FILE}" >&2; exit 2; }

TABLE="$(sed -z 's/\\\n[[:space:]]*//g' "${FILE}")"
TABLE="${TABLE#*pub const CATALOG}"
TABLE="${TABLE%%#\[cfg(test)\]*}"

block="${TABLE#*"id: \"${ID}\","}"
[ "${block}" != "${TABLE}" ] || exit 0
block="${block%%CatalogEntry \{*}"
case "${block}" in
    *$'\n'"        ${FIELD}:"*) ;;
    *) exit 0 ;;
esac
rest="${block#*$'\n'"        ${FIELD}:"}"
rest="${rest#"${rest%%[![:space:]]*}"}"
if [[ "${rest}" == None* ]]; then
    echo NONE
elif [[ "${rest}" =~ ^(Some\([[:space:]]*)?\"([^\"]*)\" ]]; then
    echo "${BASH_REMATCH[2]}"
elif [[ "${rest}" =~ ^([0-9_]+) ]]; then
    echo "${BASH_REMATCH[1]//_/}"
fi
