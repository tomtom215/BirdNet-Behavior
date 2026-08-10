# ---------------------------------------------------------------------------
# ZRAM compressed swap (optional — Pi Zero 2W and low-RAM boards)
# ---------------------------------------------------------------------------

# Install and enable a ZRAM swap device sized at half of physical RAM.
#
# ZRAM uses in-RAM compression rather than swapping to SD card, which:
#   - Dramatically reduces SD card wear (no swap writes to disk)
#   - Provides more effective working memory on Pi Zero 2W (512 MB RAM)
#   - Is transparent to the OS and BirdNet-Behavior
#
# Requires kernel >= 3.15 (all Pi models supported by BirdNET-Pi ship this).
# BirdNET-Pi equivalent: install_zram_service.sh
setup_zram() {
    info "Setting up ZRAM compressed swap…"

    # Check for zramctl (util-linux) — available on Raspberry Pi OS Bullseye+
    if ! command -v zramctl &>/dev/null; then
        warn "zramctl not found — installing util-linux…"
        pkg_install util-linux || {
            warn "Could not install util-linux ($(pkg_install_hint util-linux)). Skipping ZRAM setup."
            return 0
        }
    fi

    local mem_bytes
    mem_bytes="$(awk '/MemTotal/ {print $2 * 1024}' /proc/meminfo)"
    local zram_size=$(( mem_bytes / 2 ))   # 50% of physical RAM

    # Load the zram kernel module
    if ! lsmod | grep -q '^zram'; then
        modprobe zram num_devices=1 || {
            warn "Could not load zram module. Skipping ZRAM setup."
            return 0
        }
    fi

    local zram_dev
    zram_dev="$(zramctl --find --size "${zram_size}" --algorithm lz4 2>/dev/null)" || {
        warn "zramctl failed to allocate device. Skipping ZRAM setup."
        return 0
    }

    mkswap "${zram_dev}" &>/dev/null
    swapon --priority 100 "${zram_dev}" || {
        warn "Failed to activate ZRAM swap device. Skipping."
        return 0
    }

    success "ZRAM swap activated: ${zram_dev} ($(( zram_size / 1024 / 1024 )) MB, lz4)"

    # Persist across reboots via a systemd service unit
    local zram_service="/etc/systemd/system/zram-swap.service"
    cat > "${zram_service}" << EOF
[Unit]
Description=ZRAM compressed swap for BirdNet-Behavior
After=multi-user.target

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/bin/sh -c 'modprobe zram num_devices=1 && zramctl --find --size ${zram_size} --algorithm lz4 | xargs -I{} sh -c "mkswap {} && swapon --priority 100 {}"'
ExecStop=/bin/sh -c 'swapoff -a 2>/dev/null; zramctl --list 2>/dev/null | awk "NR>1{print \$1}" | xargs -r rmmod zram 2>/dev/null || true'

[Install]
WantedBy=multi-user.target
EOF

    if has_systemd; then
        systemctl daemon-reload
        systemctl enable zram-swap.service &>/dev/null
        success "ZRAM swap service installed and enabled (persists across reboots)."
    else
        success "ZRAM swap unit written (enable it with systemctl on a systemd host)."
    fi
}
