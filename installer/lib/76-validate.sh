# ---------------------------------------------------------------------------
# Post-install validation
#
# After an install / update / repair, confirm the result is actually healthy
# and report each check. Advisory by design: it never aborts (the install
# already happened), but a FAIL line tells the operator exactly what to fix.
# ---------------------------------------------------------------------------

# Run a command as the service user so writability/ownership checks reflect
# what the daemon will actually see (root can write everywhere; the service
# user cannot). Falls back gracefully when runuser/sudo are unavailable.
run_as_service_user() {
    if command -v runuser &>/dev/null; then
        runuser -u "${SERVICE_USER}" -- "$@"
    elif command -v sudo &>/dev/null; then
        sudo -n -u "${SERVICE_USER}" -- "$@"
    else
        "$@"
    fi
}

# Number of validation problems found, for the caller's summary line.
VALIDATION_FAILURES=0

_v_pass() { success "  check: $*"; }
_v_warn() { warn "  check: $*"; }
_v_fail() { error "  check: $*"; VALIDATION_FAILURES=$((VALIDATION_FAILURES + 1)); }

validate_install() {
    info "Validating installation…"
    VALIDATION_FAILURES=0

    # 1. Binary runs.
    if [ -x "${INSTALL_DIR}/${BINARY_NAME}" ] \
        && "${INSTALL_DIR}/${BINARY_NAME}" --version &>/dev/null; then
        _v_pass "binary executes ($("${INSTALL_DIR}/${BINARY_NAME}" --version 2>/dev/null | head -1))"
    else
        _v_fail "binary at ${INSTALL_DIR}/${BINARY_NAME} is missing or won't run"
    fi

    # 2. Service unit parses. systemd-analyze verify catches typos and a number
    #    of sandboxing mistakes before they cause a start failure.
    if [ -f "${SERVICE_FILE}" ]; then
        if command -v systemd-analyze &>/dev/null; then
            local verify_out
            if verify_out="$(systemd-analyze verify "${SERVICE_FILE}" 2>&1)"; then
                _v_pass "systemd unit verifies clean"
            else
                _v_warn "systemd-analyze verify reported: ${verify_out}"
            fi
        else
            _v_pass "service unit present (systemd-analyze not available to verify)"
        fi
    else
        _v_fail "service unit ${SERVICE_FILE} is missing"
    fi

    # 3. Data directories exist and belong to the service user.
    local d owner
    for d in "${DATA_DIR}" "${RECS_DIR}" "${MODEL_DIR}"; do
        if [ ! -d "${d}" ]; then
            _v_fail "directory missing: ${d}"
            continue
        fi
        owner="$(stat -c '%U' "${d}" 2>/dev/null || echo '?')"
        if [ "${owner}" = "${SERVICE_USER}" ]; then
            _v_pass "${d} owned by ${SERVICE_USER}"
        else
            _v_fail "${d} owned by ${owner}, expected ${SERVICE_USER} (run: install.sh repair)"
        fi
    done

    # 4. Config is readable by the daemon (service user, via group).
    if [ -f "${CONFIG_FILE}" ]; then
        if run_as_service_user test -r "${CONFIG_FILE}" 2>/dev/null; then
            _v_pass "config readable by ${SERVICE_USER}"
        else
            _v_fail "${CONFIG_FILE} not readable by ${SERVICE_USER} (run: install.sh repair)"
        fi
    fi

    # 5. Doctor preflight, run as the service user (mirrors ExecStartPre).
    local rc=0
    run_as_service_user "${INSTALL_DIR}/${BINARY_NAME}" --doctor --config "${CONFIG_FILE}" \
        &>/dev/null || rc=$?
    case "${rc}" in
        0) _v_pass "doctor preflight passed" ;;
        1) _v_warn "doctor preflight passed with warnings (run: ${BINARY_NAME} --doctor --config ${CONFIG_FILE})" ;;
        *) _v_fail "doctor preflight reported errors (run: ${BINARY_NAME} --doctor --config ${CONFIG_FILE})" ;;
    esac

    # 6. If the service is up, confirm the web port is actually listening.
    if systemctl is-active --quiet "${SERVICE_NAME}" 2>/dev/null; then
        local port="${LISTEN_ADDR##*:}"
        if command -v ss &>/dev/null && ss -ltn 2>/dev/null | grep -q ":${port}\b"; then
            _v_pass "service active and listening on port ${port}"
        else
            _v_warn "service active but port ${port} not seen listening yet (it may still be starting)"
        fi
    else
        info "  check: service not running yet (start it once an audio source is set)."
    fi

    if [ "${VALIDATION_FAILURES}" -eq 0 ]; then
        success "Validation passed."
    else
        warn "Validation found ${VALIDATION_FAILURES} problem(s) — see the check lines above."
    fi
}
