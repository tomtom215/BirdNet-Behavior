# On-device hardware acceptance test

`docs/RELEASE_PLAN.md` § 5 lists what a green CI matrix cannot establish:

> **Real Raspberry Pi hardware.** Throughput, thermals and live capture from a
> physical microphone remain unmeasured. qemu-user proved the aarch64 binary
> executes and serves; it is not the board.

`scripts/hardware-test.sh` is how that gets established. It drives the installed
systemd service on real hardware — installing from the published release exactly
as an operator would, measuring what the board actually does, then deliberately
breaking the station to prove each documented recovery path is real rather than
merely coded.

Every result is recorded with the command that produced it, in the same spirit
as the release plan's method note: *a green gate only proves what it actually
executes.*

---

## 1. What you need

| Requirement | Why |
|---|---|
| Raspberry Pi 4 or 5, **64-bit** OS | There is no armv7 binary |
| Raspberry Pi OS **Trixie** / Debian 13 (glibc ≥ 2.39) | Bookworm's glibc 2.36 cannot run the native binary — use Docker there instead |
| A USB microphone, plugged in | Half the phases are about live capture |
| ~6 GiB free disk | 34 MB binary + ~541 MB model + recordings + test ballast |
| Network (for the install only) | Everything after the fetch runs offline |
| An ordinary login account with `sudo` | **Do not run the harness as root** — the station's data directory is derived from `$HOME` |

A wired SSH session is easier than wifi, because one phase deliberately drops
the network. If you are on wifi over SSH, read § 4 first — the harness arms a
systemd recovery timer before it drops the link, so the box comes back even if
your session dies with it.

---

## 2. Running it

```bash
# On the Pi, as your normal user (not root, not sudo).
# Run it inside tmux: the netloss phase drops the network and the reboot phase
# reboots the board — either will kill a bare SSH session and the run with it.
tmux new -s hwtest

curl -fsSL https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/scripts/hardware-test.sh \
  -o hardware-test.sh
chmod +x hardware-test.sh
./hardware-test.sh
```

(If `tmux` is not installed: `sudo apt install -y tmux`. The harness detects an
SSH session outside tmux and warns before it starts. Reattach after a drop with
`tmux attach -t hwtest`.)

The full suite takes roughly **45–70 minutes**, most of it the install (a
~541 MB model over wifi) and the 10-minute performance sampling window.

Useful variants:

```bash
./hardware-test.sh --list             # show the phase ids
./hardware-test.sh --safe             # skip everything destructive
./hardware-test.sh --phase perf       # re-run one phase
./hardware-test.sh --skip install     # run everything except one phase
./hardware-test.sh --resume           # continue after the reboot phase
BIRDNET_PERF_MINUTES=30 ./hardware-test.sh --phase perf   # longer thermal soak
```

> **Testing a binary you installed yourself?** Use `--skip install`. The
> install phase fetches the **published release** and would overwrite it, so
> everything after that point would describe a different build than the one you
> meant to test. Skipped phases are recorded as skipped in the state file, so
> the `--resume` after the reboot does not quietly run them either.

The reboot phase reboots the board. When it comes back, reconnect and run
`./hardware-test.sh --resume` — the harness remembers which phases already
passed and picks up the post-reboot assertions.

Results land in `./birdnet-hwtest-<timestamp>/`:

| File | Contents |
|---|---|
| `report.md` | The summary to read and to share |
| `results.jsonl` | One JSON object per check, machine-readable |
| `env.txt` | Full board/OS/peripheral inventory |
| `install.log` | Complete installer output |
| `perf-samples.csv` | Temperature / RSS / inference-count time series |
| `journal-<phase>.log` | Service journal captured at each fault injection |
| `doctor.txt`, `verify-extension.txt` | Diagnostic output verbatim |

Exit status is non-zero if any check failed.

---

## 3. What each phase proves

| Phase | Establishes |
|---|---|
| `env` | Board model, glibc floor, capture device present, **undervoltage/throttle register**, NTP sync, free disk |
| `install` | The published one-liner completes on a clean box, and how long it takes |
| `verify` | Binary runs, doctor's verdict, service active, health endpoint, **`--verify-extension` under `unshare -rn`**, systemd hardening actually applied |
| `capture` | A real capture subprocess is running, segments reach the tmpfs watch dir, recordings persist, and the mic is not delivering digital silence |
| `detect` | The reference Eurasian Magpie recording, pushed through the watch directory, produces a `Pica pica` detection on the API — real inference against the 11k-species model, on this board |
| `perf` | **Mean inference latency per 3 s chunk**, peak SoC temperature, throttle register under load, RSS against the unit's `MemoryHigh=768M` |
| `web` | Every documented endpoint answers; the dashboard is reachable from the LAN; whether `/admin` is open without a password |
| `watchdog` | `SIGSTOP` the daemon → systemd's watchdog kills and restarts it, and the station serves again |
| `unplug` | Pull the mic → gauge drops to 0, daemon survives; plug it back → **recovers unattended** via capped backoff |
| `netloss` | 60 s with no network → daemon stays up, loopback still serves, no panic, connectivity returns |
| `diskfull` | Filesystem pushed past the 95 % purge threshold **and below doctor's 1 GiB floor** → `--doctor` still permits startup, the service **restarts** under that pressure, disk manager reacts, daemon survives, no panic |
| `dbcorrupt` | 8 KiB of random bytes over the SQLite header → `--check-db` detects it, the station recovers on restart, and ends healthy |
| `duckdb` | Same against the derived analytics store → it rebuilds rather than refusing to start |
| `reboot` | Cold reboot → service auto-starts, dashboard serves, **capture resumes** |

### The two headline numbers

- **Mean inference latency.** A 3-second chunk must classify in well under
  3000 ms or the station cannot keep up with a live microphone in real time.
  The harness fails `perf.realtime` if it does not.
- **Peak SoC temperature.** A Pi 4 begins soft-throttling at 80 °C. The harness
  warns above that, because a bench result at 20 °C ambient is not a July
  result in a sealed enclosure.

---

## 4. Safety

The destructive phases are genuinely destructive; each one arms its undo
*before* it breaks anything.

- **Disk fill.** A ballast file is sized to satisfy both thresholds at once —
  at least 96 % used *and* under 1 GiB free, since the purge fires on a
  percentage while doctor grades in absolute bytes — while never consuming the
  last 200 MiB. It is removed by an `EXIT`/`INT`/`TERM` trap, and the phase
  restarts the service afterwards if its own restart test left the unit parked.
  If the filesystem cannot reach the target safely, the phase skips.

- **Ctrl-C stops the run.** Worth stating because it did not always: a bash
  trap handler that merely returns does *not* end the script — execution
  resumes where the signal landed. Signals now get a handler that cleans up and
  then exits 130, so Ctrl-C frees the ballast, resumes any `SIGSTOP`ped
  process, and stops, rather than cleaning up and carrying on into the next
  fault injection.
- **Network drop.** A `systemd-run --on-active=75` timer restores connectivity
  before the link goes down, so a dropped SSH session recovers on its own. On a
  NetworkManager box the harness uses `nmcli networking off` rather than
  `ip link set … down`, which NetworkManager would immediately undo — that
  would make the phase a no-op that reports success.
- **Database corruption.** The harness takes its own copy of `birds.db` (and
  the WAL/SHM sidecars) into the results directory first, and restores it if
  the station does not recover on its own. It also runs `--backup-db` so the
  product's own recovery path has something to restore from.
- **Watchdog.** If the test fails, the `SIGSTOP`ped process is sent `SIGCONT`
  by the exit trap rather than left frozen.

Run `--safe` to skip all of it.

---

## 5. Reporting results

`report.md` is written to be pasted directly into an issue or a release note.
It carries the board, OS, glibc, version under test, the pass/fail/warn/skip
tally, the measured numbers, and a per-phase finding list with a
"Failures to triage" section at the end.

When a phase fails, the matching `journal-<phase>.log` holds the service
journal from that moment — start there.
