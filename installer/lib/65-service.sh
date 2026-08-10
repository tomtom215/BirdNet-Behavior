# ---------------------------------------------------------------------------
# Install the systemd service unit
# ---------------------------------------------------------------------------

# Decide the dashboard bind address, PRESERVING it across re-runs so a repair or
# update never silently re-hides a LAN-exposed dashboard back on localhost.
# Precedence (highest first):
#   1. BIRDNET_LISTEN in the environment (explicit override; already in LISTEN_ADDR)
#   2. BIRDNET_LISTEN= in the config file (the operator-editable source of truth)
#   3. --listen in an existing service unit (carry the previous choice forward)
#   4. the interactive prompt / default already in LISTEN_ADDR (fresh installs)
resolve_listen_addr() {
    [ -n "${BIRDNET_LISTEN:-}" ] && return 0

    # The `|| true` keeps a no-match grep (exit 1) from tripping `set -o pipefail`
    # + `set -e` and aborting the whole installer — the common case is a config
    # with no uncommented BIRDNET_LISTEN.
    local from_cfg=""
    if [ -f "${CONFIG_FILE}" ]; then
        from_cfg="$(grep -E '^[[:space:]]*BIRDNET_LISTEN[[:space:]]*=' "${CONFIG_FILE}" 2>/dev/null \
            | tail -1 | cut -d= -f2- | tr -d '[:space:]' || true)"
    fi
    if [ -n "${from_cfg}" ]; then
        LISTEN_ADDR="${from_cfg}"
        info "Dashboard bind address from config: ${LISTEN_ADDR}"
        return 0
    fi

    if [ -f "${SERVICE_FILE}" ]; then
        local from_unit
        from_unit="$(grep -oE -- '--listen [^ ]+' "${SERVICE_FILE}" 2>/dev/null \
            | head -1 | awk '{print $2}' || true)"
        if [ -n "${from_unit}" ] && [ "${from_unit}" != "${LISTEN_ADDR}" ]; then
            LISTEN_ADDR="${from_unit}"
            info "Preserving dashboard bind address from the existing unit: ${LISTEN_ADDR}"
        fi
    fi
}

install_service() {
    info "Installing systemd service…"

    cat > "${SERVICE_FILE}" <<EOF
[Unit]
Description=BirdNet-Behavior bird detection and analytics
Documentation=https://github.com/${REPO}
# Wait for the network stack AND sound subsystem before launching. The
# detection daemon needs both; running before them just causes an
# avoidable restart loop on slow-booting hardware (USB enumeration on Pi).
After=network-online.target sound.target time-sync.target
Wants=network-online.target
# Don't enter a tight restart loop. If 5 restarts happen inside 5 min the
# unit is marked failed and stays down for operator review (visible in
# the web UI's health page once the service comes back).
StartLimitBurst=5
StartLimitIntervalSec=300

[Service]
# Type=notify pairs with sd_notify in src/sd_notify.rs:
#   - READY=1 when the web server has bound its socket
#   - WATCHDOG=1 periodic pings keep the watchdog happy
#   - STOPPING=1 on graceful shutdown
Type=notify
NotifyAccess=main
User=${SERVICE_USER}

# Serve the bundled operator manual (mdBook) at /help/*. install.sh installs it
# from the release tarball to ${HELP_DIR}; harmless if absent on older releases —
# the ServeDir simply returns 404 for /help, exactly as before.
Environment=BNB_HELP_DIR=${HELP_DIR}

# Recreate the ephemeral stream/watch dir before anything else runs. With
# PrivateTmp=yes (below) the service gets a FRESH, EMPTY /tmp on every start,
# so /tmp/birdnet-stream never survives a restart — create it here so the
# file-watcher has somewhere to attach and the doctor preflight sees a
# writable recordings dir. The binary also creates it at startup; doing it
# here keeps older binaries working too.
#
# IMPORTANT: ${STREAM_DIR} must NOT appear in ReadWritePaths= below. PrivateTmp
# mounts a new tmpfs over /tmp, and bind-mounting a path *beneath* that new
# mount fails namespace setup with "${STREAM_DIR}: No such file or directory"
# (which is exactly the start failure this avoids). The private /tmp is already
# writable, so the watch dir does not need to be in ReadWritePaths.
ExecStartPre=/bin/mkdir -p ${STREAM_DIR}

# Preflight: run the doctor before starting the main service so a broken
# install fails fast with an actionable report in the journal, rather than
# entering a restart loop that fills the disk with logs.
# Exit 0 (pass) or 1 (warnings only) are both accepted — only exit 2
# (errors that will prevent operation) keeps the service from starting.
ExecStartPre=/bin/sh -c '${INSTALL_DIR}/${BINARY_NAME} --doctor --config ${CONFIG_FILE} || [ \$? -le 1 ]'
# DuckDB behavioral analytics is compiled into every release binary and enabled
# here by default (the database is created on first run). To run without it
# (e.g. on a very low-RAM board), remove the --analytics-db flag below.
ExecStart=${INSTALL_DIR}/${BINARY_NAME} --config ${CONFIG_FILE} --listen ${LISTEN_ADDR} --watch-dir ${STREAM_DIR} --image-cache-dir ${IMAGE_CACHE_DIR} --analytics-db ${DATA_DIR}/analytics.db

# Restart policy. panic=abort means panics show up as SIGABRT exits;
# Restart=always covers panics, OOM kills, and any non-zero exit.
Restart=always
RestartSec=10
# Generous startup budget so a first-run model download / DB migration
# doesn't trip the watchdog while it is still legitimately working.
TimeoutStartSec=900
# Allow graceful shutdown to drain WAL and finish outstanding HTTP
# requests; SIGTERM is the friendly signal, SIGKILL is the fallback.
TimeoutStopSec=30
KillSignal=SIGTERM
KillMode=mixed
SendSIGKILL=yes

# Watchdog: src/sd_notify.rs pings every WatchdogSec/2.
# 120 s window is plenty: a healthy daemon pings every ~60 s.
WatchdogSec=120

# Resource ceilings — cap a runaway process without starving the workload.
# The bundled DuckDB analytics engine is on by default and its queries can be
# memory-hungry under load; 1 GiB leaves that headroom (the FP32 model is
# mmap'd, so its pages are reclaimable and don't count as anonymous RSS). On a
# multi-GB Pi this is the binding limit; on a 512 MB board physical RAM + zram
# bind first, so raising the cgroup ceiling here is harmless there.
MemoryHigh=768M
MemoryMax=1G
TasksMax=512
LimitNOFILE=65536
LimitNPROC=256
# Recover gracefully under memory pressure.
OOMScoreAdjust=200
OOMPolicy=stop

# ── Filesystem isolation ─────────────────────────────────────────────────
# Read-only access to the rest of the filesystem; explicit write paths.
# ${STREAM_DIR} is intentionally absent — it lives in the PrivateTmp /tmp
# (see ExecStartPre= above); listing it here would break namespace setup.
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=${DATA_DIR} ${CONFIG_DIR} /run /var/log
PrivateTmp=yes
ProtectKernelLogs=yes
ProtectKernelModules=yes
ProtectKernelTunables=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
ProtectProc=invisible
# Deliberately NOT ProcSubset=pid. The Station Health "Vitals" read the
# system-wide /proc files — /proc/stat and /proc/cpuinfo (CPU %, core count)
# and /proc/meminfo (memory), via the sysinfo crate, plus /proc/uptime.
# ProcSubset=pid hides exactly those non-process files, which made the
# dashboard report 0 CPU cores and 0 B memory while temperature/disk (read
# from /sys and statvfs) still worked. Leave /proc at the default (all);
# ProtectProc=invisible above still hides other users' processes.
# Block-listed kernel surfaces; we don't need them.
RestrictSUIDSGID=yes
RestrictRealtime=yes
RestrictNamespaces=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
NoNewPrivileges=yes
# Drop every capability from the bounding set — a non-root network + audio
# service needs none, and with NoNewPrivileges it can never regain them.
CapabilityBoundingSet=
# Files the service creates (database, recordings) are group-readable at most,
# never world-readable.
UMask=0027
# Restrict sockets to what a web service + journald + local-IP lookup require.
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK
SystemCallArchitectures=native
# Permit only POSIX, file I/O, networking, and signals — explicitly
# excludes things like raw_io / module_load / ptrace / mount / reboot.
SystemCallFilter=@system-service
SystemCallFilter=~@privileged @resources @mount @debug @cpu-emulation @obsolete @reboot @swap @raw-io @clock @module

# Audio access — must keep these capability sets / device mounts.
#
# DeviceAllow= resolves a path to a *device node*. /dev/snd is a DIRECTORY, so
# "DeviceAllow=/dev/snd rw" matches nothing: with DevicePolicy=closed every ALSA
# node (/dev/snd/pcmC1D0c, controlC1, …) stayed denied, and microphone capture
# could never work on a bare-metal install. arecord still exec'd successfully —
# so the daemon logged "started microphone capture" — and only then failed the
# PCM open with "audio open error: No such file or directory", which the
# supervisor saw as a stalled source and restarted forever.
#
# char-alsa is systemd's documented subsystem form and is what actually grants
# the nodes. Verified on a Raspberry Pi 4 (Pi OS Trixie, USB mic on card 1) with
# an A/B under systemd-run: "/dev/snd rw" fails to open the device, "char-alsa
# rw" records normally. RTSP stations were unaffected — ffmpeg never touches
# /dev/snd — which is why this survived from v0.6.0 to v0.11.0 unnoticed.
SupplementaryGroups=audio
DeviceAllow=char-alsa rw
DevicePolicy=closed

# ── Logging ──────────────────────────────────────────────────────────────
StandardOutput=journal
StandardError=journal
SyslogIdentifier=birdnet-behavior
# Cap journal volume so a chatty failure mode can't exhaust the disk on
# a Pi with a small SD card.
LogRateLimitIntervalSec=30
LogRateLimitBurst=1000

[Install]
WantedBy=multi-user.target
EOF

    if has_systemd; then
        systemctl daemon-reload
        systemctl enable birdnet-behavior.service
        success "Service installed and enabled (Type=notify, hardened, watchdog active)."
    else
        success "Service unit written to ${SERVICE_FILE} (Type=notify, hardened, watchdog active)."
        warn "systemd is not running here — not enabling/starting the unit."
        warn "  On a systemd host, finish with:"
        warn "    sudo systemctl daemon-reload && sudo systemctl enable --now birdnet-behavior"
    fi
}
