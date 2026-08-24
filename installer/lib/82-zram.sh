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
    local loaded_modules
    loaded_modules="$(lsmod 2>/dev/null || true)"
    if ! grep -q '^zram' <<<"${loaded_modules}"; then
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

    # Persist across reboots via a systemd service unit.
    #
    # ExecStop deserves a note. It used to begin `swapoff -a`, which is
    # documented as "disable all swaps from /proc/swaps" — every swap on the
    # machine, not the zram device this unit made. Raspberry Pi OS enables
    # dphys-swapfile by default, so stopping this unit (on shutdown, on
    # `systemctl stop zram-swap`, during an uninstall) silently switched off the
    # operator's real swap on exactly the low-RAM boards this unit exists to
    # help. It then piped device paths into `rmmod`, which takes a module name,
    # so `rmmod /dev/zram0` failed on every run and `|| true` hid it.
    #
    # Now: only /dev/zram* are swapped off, and the module is unloaded by name.
    # This still cannot distinguish a zram device made by another provider
    # (zram-tools) from ours — the unit records no device id — but at shutdown
    # that is the difference between touching zram and touching everything.
    local zram_service="/etc/systemd/system/zram-swap.service"
    cat > "${zram_service}" << EOF
[Unit]
Description=ZRAM compressed swap for BirdNet-Behavior
After=multi-user.target

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/bin/sh -c 'modprobe zram num_devices=1 && zramctl --find --size ${zram_size} --algorithm lz4 | xargs -I{} sh -c "mkswap {} && swapon --priority 100 {}"'
ExecStop=/bin/sh -c 'for d in /dev/zram*; do [ -b "\$d" ] && swapoff "\$d" 2>/dev/null; done; rmmod zram 2>/dev/null || true'

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
