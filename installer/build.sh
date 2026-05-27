#!/usr/bin/env bash
# installer/build.sh — assemble the modular installer sources into the single,
# self-contained install.sh that ships to operators.
#
# Why a build step?
#   install.sh is a `curl … | sudo bash` one-liner, is checksummed in
#   SHA256SUMS, and is bundled inside every release tarball — so the *shipped*
#   artifact has to be one self-contained file. We still want small,
#   single-responsibility sources that are easy to read, test, and debug, so
#   the sources live in installer/lib/*.sh and this script concatenates them
#   (in filename order) into ../install.sh.
#
# Usage:
#   installer/build.sh            # (re)generate ../install.sh
#   installer/build.sh --check    # verify ../install.sh is in sync (CI gate)
#   installer/build.sh --stdout   # print the assembled script to stdout
#
# Editing rule: NEVER edit install.sh by hand. Edit installer/lib/*.sh and
# re-run this script. CI runs `--check` and fails if the two diverge.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIB_DIR="${SCRIPT_DIR}/lib"
OUT_FILE="${SCRIPT_DIR}/../install.sh"

if [ ! -d "${LIB_DIR}" ]; then
    echo "build.sh: missing module directory ${LIB_DIR}" >&2
    exit 1
fi

# Assemble to stdout. Modules are concatenated in LC_ALL=C filename order; the
# numeric NN- prefixes encode load order (config before logging before main).
assemble() {
    local part rel

    cat <<'BANNER'
#!/usr/bin/env bash
# =============================================================================
#  install.sh — GENERATED FILE. DO NOT EDIT.
#
#  This file is assembled from installer/lib/*.sh by installer/build.sh.
#  To change the installer, edit the relevant module under installer/lib/ and
#  run `installer/build.sh`. CI verifies this file stays in sync.
# =============================================================================
BANNER

    while IFS= read -r part; do
        rel="installer/lib/$(basename "${part}")"
        printf '\n# ===== %s =====\n' "${rel}"
        # Drop a leading shebang from a module (there should be none, but keep
        # the assembled file single-shebang regardless). Everything else is
        # emitted verbatim so line-for-line debugging maps back to the module.
        sed '1{/^#!/d;}' "${part}"
    done < <(find "${LIB_DIR}" -maxdepth 1 -type f -name '*.sh' | LC_ALL=C sort)
}

case "${1:-}" in
    --stdout)
        assemble
        ;;
    --check)
        tmp="$(mktemp)"
        trap 'rm -f "${tmp}"' EXIT
        assemble >"${tmp}"
        if [ ! -f "${OUT_FILE}" ]; then
            echo "build.sh --check: ${OUT_FILE} does not exist — run installer/build.sh" >&2
            exit 1
        fi
        if ! diff -u "${OUT_FILE}" "${tmp}"; then
            echo >&2
            echo "build.sh --check: install.sh is OUT OF SYNC with installer/lib/*.sh." >&2
            echo "Run: installer/build.sh   (then commit the regenerated install.sh)" >&2
            exit 1
        fi
        echo "install.sh is in sync with installer/lib/*.sh"
        ;;
    "")
        tmp="$(mktemp)"
        trap 'rm -f "${tmp}"' EXIT
        assemble >"${tmp}"
        chmod 0755 "${tmp}"
        mv "${tmp}" "${OUT_FILE}"
        trap - EXIT
        echo "Wrote ${OUT_FILE} from ${LIB_DIR}/*.sh"
        ;;
    *)
        echo "Usage: installer/build.sh [--check|--stdout]" >&2
        exit 2
        ;;
esac
