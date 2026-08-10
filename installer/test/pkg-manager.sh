#!/usr/bin/env bash
# installer/test/pkg-manager.sh — exercise the installer's package-manager
# layer against real distributions, in containers.
#
# Why this exists: every auto-install path used to be gated on `command -v
# apt-get`, so on Fedora/Arch/openSUSE the installer printed an apt command
# that cannot run. The replacement (detect_pkg_mgr / pkg_name_for /
# pkg_install / pkg_install_hint in installer/lib/30-platform.sh) is exactly
# the kind of code that looks obviously right and is wrong in the details —
# package names differ, and a non-zero return under `set -e` aborts the
# install. So it is checked by installing the real packages, on the real
# distros, and asserting the tool lands on PATH.
#
# The matrix, and what it verified when written:
#
#   Debian trixie   apt     ffmpeg        alsa-utils qrencode util-linux
#   Fedora 41       dnf     ffmpeg-free   alsa-utils qrencode util-linux
#   Arch            pacman  ffmpeg        alsa-utils qrencode util-linux
#   openSUSE TW     zypper  ffmpeg        alsa-utils qrencode util-linux
#
# ffmpeg is the only name that differs: Fedora's main repositories ship
# `ffmpeg-free`, with the unencumbered `ffmpeg` in RPM Fusion, which an
# application installer has no business enabling on someone's machine.
#
# Usage:
#   installer/test/pkg-manager.sh              # whole matrix
#   installer/test/pkg-manager.sh fedora       # one distro
#   installer/test/pkg-manager.sh nopm         # unknown-distro fallback
#
# Requires docker and network access. Not wired into CI: it pulls four base
# images and installs packages from four mirror networks, which is a lot of
# minutes and four more things that can be flaky. Run it when touching
# 30-platform.sh or the ensure_capture_* paths.
#
# Behind a TLS-intercepting proxy (some sandboxes), export PROXY_CA=/path/to/
# ca-bundle.crt and the Debian leg will trust it; distro mirrors reached over
# plain http may still be unreachable and that is the proxy's limit, not the
# installer's.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PROXY_CA="${PROXY_CA:-}"
FAILED=0

if ! command -v docker >/dev/null 2>&1; then
    echo "docker is required" >&2
    exit 1
fi

# distro-key | image | expected manager | expected ffmpeg package
MATRIX=(
    "debian|debian:trixie-slim|apt|ffmpeg"
    "fedora|fedora:41|dnf|ffmpeg-free"
    "arch|archlinux:latest|pacman|ffmpeg"
    "suse|opensuse/tumbleweed:latest|zypper|ffmpeg"
)

# Prepare the in-container script once; it is the same for every distro.
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

cat > "${WORK}/assert.sh" <<'INNER'
#!/usr/bin/env bash
# $1 = expected package manager, $2 = expected ffmpeg package name
set -u
EXPECT_MGR="$1"; EXPECT_FFMPEG="$2"; F=0
ok()  { echo "  PASS  $*"; }
bad() { echo "  FAIL  $*"; F=1; }

# shellcheck disable=SC1091
. /repo/installer/lib/10-config.sh
. /repo/installer/lib/20-log.sh
. /repo/installer/lib/30-platform.sh
. /repo/installer/lib/45-preflight.sh
# 10-config.sh enables `set -euo pipefail`; these assertions call functions
# that intentionally return non-zero.
set +e

# shellcheck disable=SC1091
echo "=== $(. /etc/os-release 2>/dev/null; echo "${PRETTY_NAME:-unknown}") ==="

detect_pkg_mgr
[ "${PKG_MGR}" = "${EXPECT_MGR}" ] && ok "detect_pkg_mgr -> ${PKG_MGR}" \
    || bad "detect_pkg_mgr -> '${PKG_MGR}', expected '${EXPECT_MGR}'"

got="$(pkg_name_for ffmpeg)"
[ "${got}" = "${EXPECT_FFMPEG}" ] && ok "pkg_name_for ffmpeg -> ${got}" \
    || bad "pkg_name_for ffmpeg -> '${got}', expected '${EXPECT_FFMPEG}'"

for g in alsa-utils qrencode util-linux; do
    got="$(pkg_name_for "${g}")"
    [ "${got}" = "${g}" ] && ok "pkg_name_for ${g} -> ${got}" || bad "pkg_name_for ${g} -> '${got}'"
done

hint="$(pkg_install_hint coreutils tar curl)"
case "${EXPECT_MGR}" in
    apt) want="apt-get" ;; dnf) want="dnf" ;;
    pacman) want="pacman" ;; zypper) want="zypper" ;;
esac
case "${hint}" in
    *"${want}"*) ok "pkg_install_hint -> ${hint}" ;;
    *)           bad "pkg_install_hint -> '${hint}', expected it to use ${want}" ;;
esac

# The point of the whole exercise: does the tool actually arrive?
if command -v arecord &>/dev/null; then
    bad "arecord already present — this image cannot prove the install path"
else
    ok "arecord absent beforehand (valid subject)"
    if pkg_install alsa-utils && command -v arecord &>/dev/null; then
        ok "pkg_install alsa-utils -> $(command -v arecord)"
    else
        bad "pkg_install alsa-utils did not put arecord on PATH"
    fi
fi

ensure_capture_tool ffmpeg ffmpeg "RTSP capture" 2>&1 | sed 's/^/        /'
command -v ffmpeg &>/dev/null && ok "ensure_capture_tool ffmpeg -> $(command -v ffmpeg)" \
    || bad "ensure_capture_tool ffmpeg left ffmpeg absent"

pkg_install qrencode >/dev/null 2>&1
command -v qrencode &>/dev/null && ok "pkg_install qrencode -> $(command -v qrencode)" \
    || bad "pkg_install qrencode did not land"
pkg_install util-linux >/dev/null 2>&1
command -v zramctl &>/dev/null && ok "pkg_install util-linux -> $(command -v zramctl)" \
    || bad "pkg_install util-linux did not put zramctl on PATH"

echo "=== $([ "${F}" = 0 ] && echo ALL-PASS || echo FAILURES) ==="
exit "${F}"
INNER

cat > "${WORK}/nopm.sh" <<'INNER'
#!/usr/bin/env bash
# Unknown distro: no supported package manager on PATH at all.
set -u
for m in apt-get dnf pacman zypper; do
    if p="$(command -v "$m" 2>/dev/null)"; then mv "$p" "$p.hidden"; fi
done
# shellcheck disable=SC1091
. /repo/installer/lib/10-config.sh
. /repo/installer/lib/20-log.sh
. /repo/installer/lib/30-platform.sh
. /repo/installer/lib/45-preflight.sh
set +e
F=0
ok()  { echo "  PASS  $*"; }
bad() { echo "  FAIL  $*"; F=1; }

echo "=== no supported package manager ==="
detect_pkg_mgr
[ -z "${PKG_MGR}" ] && ok "detect_pkg_mgr -> '' (none recognised)" \
    || bad "PKG_MGR='${PKG_MGR}', expected empty"

pkg_install alsa-utils && bad "pkg_install returned 0 with no package manager" \
    || ok "pkg_install returns non-zero"

hint="$(pkg_install_hint coreutils tar curl)"
case "${hint}" in
    *"your distribution's package manager"*) ok "hint degrades: ${hint}" ;;
    *) bad "hint was '${hint}'" ;;
esac

out="$(ensure_capture_tool ffmpeg ffmpeg 'RTSP capture' 2>&1)"; rc=$?
[ "${rc}" -ne 0 ] && ok "ensure_capture_tool returns ${rc}, no crash" || bad "returned 0"
printf '%s\n' "${out}" | sed 's/^/        /'
case "${out}" in
    *apt-get*) bad "still suggests apt-get on a box without it" ;;
    *)         ok "suggests no package manager this box lacks" ;;
esac
# Prose is not a command: chaining it with && would look runnable and not be.
case "${out}" in
    *"package manager && sudo"*) bad "prose hint chained with &&" ;;
    *)                           ok "prose hint not presented as a command" ;;
esac

echo "=== $([ "${F}" = 0 ] && echo ALL-PASS || echo FAILURES) ==="
exit "${F}"
INNER

[ -n "${PROXY_CA}" ] && cp "${PROXY_CA}" "${WORK}/proxy-ca.crt"

# A proxy on the host's loopback is unreachable from a container's own network
# namespace, so share the host's when (and only when) that is the situation.
NET_ARGS=()
case "${https_proxy:-}" in
    *127.0.0.1*|*localhost*) NET_ARGS=(--network host) ;;
esac

run_in() { # run_in <image> <setup-key> <args...>
    local image="$1" key="$2"; shift 2
    docker run --rm \
        "${NET_ARGS[@]+"${NET_ARGS[@]}"}" \
        -e "https_proxy=${https_proxy:-}" -e "http_proxy=${http_proxy:-}" \
        -v "${REPO_ROOT}:/repo:ro" -v "${WORK}:/w" \
        "${image}" bash -c "
            set -u
            case '${key}' in
              debian)
                if [ -f /w/proxy-ca.crt ]; then
                  sed -i 's|http://|https://|g' /etc/apt/sources.list.d/*.sources 2>/dev/null
                  cp /w/proxy-ca.crt /proxy-ca.pem
                  printf 'Acquire::https::CaInfo \"/proxy-ca.pem\";\n' > /etc/apt/apt.conf.d/99proxy-ca
                fi
                apt-get update -qq >/dev/null 2>&1 ;;
              suse)
                if [ -f /w/proxy-ca.crt ]; then
                  sed -i 's|http://download.opensuse.org|https://download.opensuse.org|' /etc/zypp/repos.d/*.repo
                  zypper --non-interactive removerepo repo-openh264 >/dev/null 2>&1
                fi
                zypper --non-interactive --gpg-auto-import-keys refresh >/dev/null 2>&1 ;;
              fedora)
                # http-only third-party repo; unreachable behind an https-only proxy
                [ -f /w/proxy-ca.crt ] && rm -f /etc/yum.repos.d/fedora-cisco-openh264.repo ;;
            esac
            exec bash $*
        "
}

only="${1:-}"

if [ -z "${only}" ] || [ "${only}" = "nopm" ]; then
    run_in "debian:trixie-slim" debian "/w/nopm.sh" || FAILED=1
fi

for row in "${MATRIX[@]}"; do
    IFS='|' read -r key image mgr ffmpeg_pkg <<<"${row}"
    [ -n "${only}" ] && [ "${only}" != "${key}" ] && continue
    [ "${only}" = "nopm" ] && continue
    echo "##### ${image}"
    run_in "${image}" "${key}" "/w/assert.sh ${mgr} ${ffmpeg_pkg}" || FAILED=1
done

echo
[ "${FAILED}" = 0 ] && echo "pkg-manager matrix: ALL-PASS" || echo "pkg-manager matrix: FAILURES"
exit "${FAILED}"
