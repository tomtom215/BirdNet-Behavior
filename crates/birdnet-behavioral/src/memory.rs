//! Sizing `DuckDB`'s buffer pool to the memory this process will actually get
//! (`G-33`).
//!
//! # What was wrong
//!
//! The buffer-pool cap was a flat `256MB` whatever the machine. On the hardware
//! the shipped systemd unit is written for — `MemoryMax=1G` — that is a quarter
//! of the budget and reasonable. On a 512 MB board it is **half of physical
//! RAM** for one subsystem, alongside the model, the web server and the OS; and
//! on a 256 MB board it is a cap that can never be honoured. `DuckDB` treats
//! the limit as permission to use that much, so the flat default is a
//! standing invitation to be OOM-killed on the smallest boards this project
//! targets.
//!
//! # What this does
//!
//! Sizes the pool as a fraction of the *effective ceiling* — the smaller of
//! physical RAM and any cgroup limit this process is under — and refuses
//! analytics outright when even the floor will not fit, with a reason, rather
//! than starting something that will be killed mid-query at three in the
//! morning.
//!
//! # Where the numbers come from
//!
//! Both are anchored rather than chosen:
//!
//! * **The fraction is a quarter**, because that is the proportion the shipped
//!   unit already implies: `MemoryMax=1G` with a `256MB` pool. A station on
//!   that unit therefore gets exactly the limit it gets today, and only smaller
//!   boards see a change — which is the point.
//! * **The floor is 64 MiB**, from a measurement rather than a guess. A
//!   sessionisation shaped like `queries.rs`'s — `lag` and a running sum over
//!   1.5 M detections partitioned by species, then aggregated — was run at
//!   descending limits on DuckDB 1.5: it failed with *Out of Memory* at 8, 16
//!   and 32 MiB and succeeded from 48 MiB up. 64 MiB is the next step above the
//!   smallest observed working value.
//!
//! What is **not** measured, and is stated here rather than implied: the rest
//! of the process's footprint on a Raspberry Pi. That would need a Pi. So this
//! sizes a *proportion*, not a budget — it makes the analytics engine's share
//! scale with the machine, and it does not claim to know that the remainder is
//! enough. What would falsify the fraction is a station within the ceiling that
//! is still OOM-killed with the pool at a quarter of it; the answer then is a
//! smaller fraction, not a different mechanism.

/// The share of the effective ceiling the buffer pool may use.
///
/// One quarter — see the module header for why this number and not another.
const POOL_SHARE_DENOMINATOR: u64 = 4;

/// Smallest pool worth starting, in MiB. Measured; see the module header.
pub const MIN_POOL_MIB: u64 = 64;

/// Largest pool this sizing will hand out, in MiB.
///
/// The flat default this replaces. A machine with more memory than the unit's
/// budget does not get a bigger pool by accident: an operator who wants one
/// says so with `BIRDNET_DUCKDB_MEMORY_LIMIT`, which is honoured verbatim.
pub const MAX_POOL_MIB: u64 = 256;

// The module header claims the fraction is *derived* from the shipped unit —
// `MemoryMax=1G` alongside a `256MB` pool — rather than chosen. That claim is
// this line, so a change to either number is a build failure rather than a
// sentence that has quietly stopped being true.
//
// It is here because the test that looks like it covers this
// (`the_shipped_units_budget_reproduces_todays_default`) does not: at a 1 GiB
// ceiling the *cap* alone produces 256 MiB, so halving the fraction still
// passes it. Measured, not assumed — that mutation was run.
const _: () = assert!(1024 / POOL_SHARE_DENOMINATOR == MAX_POOL_MIB);

/// What the sizing decided, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Budget {
    /// The operator named a limit; it is used as given.
    Configured {
        /// The literal, as written.
        limit: String,
    },
    /// Sized from the effective ceiling.
    Sized {
        /// The pool, in MiB.
        pool_mib: u64,
        /// The ceiling it was sized from, in MiB.
        ceiling_mib: u64,
        /// Where that ceiling came from.
        source: CeilingSource,
    },
    /// The machine cannot carry the smallest usable pool.
    TooSmall {
        /// The effective ceiling, in MiB.
        ceiling_mib: u64,
        /// Where it came from.
        source: CeilingSource,
    },
    /// Nothing could be detected, so the flat default stands.
    ///
    /// Not an error: a platform whose memory this cannot read is a platform
    /// where guessing would be worse than keeping the behaviour that has
    /// shipped for every release so far.
    Undetected,
}

impl Budget {
    /// The `DuckDB` memory-limit literal to set, or `None` when analytics
    /// should not be started at all.
    #[must_use]
    pub fn limit(&self) -> Option<String> {
        match self {
            Self::Configured { limit } => Some(limit.clone()),
            Self::Sized { pool_mib, .. } => Some(format!("{pool_mib}MB")),
            Self::Undetected => Some(format!("{MAX_POOL_MIB}MB")),
            Self::TooSmall { .. } => None,
        }
    }

    /// One line an operator can act on.
    #[must_use]
    pub fn explain(&self) -> String {
        match self {
            Self::Configured { limit } => {
                format!("analytics buffer pool set to {limit} by BIRDNET_DUCKDB_MEMORY_LIMIT")
            }
            Self::Sized {
                pool_mib,
                ceiling_mib,
                source,
            } => format!(
                "analytics buffer pool sized to {pool_mib} MiB, a quarter of the {ceiling_mib} MiB \
                 this process may use ({})",
                source.describe()
            ),
            Self::TooSmall {
                ceiling_mib,
                source,
            } => format!(
                "analytics is off: this process may use {ceiling_mib} MiB ({}), and the smallest \
                 usable buffer pool is {MIN_POOL_MIB} MiB — a quarter of that ceiling would be \
                 {} MiB. Raise the memory limit, or run analytics on another machine",
                source.describe(),
                ceiling_mib / POOL_SHARE_DENOMINATOR
            ),
            Self::Undetected => format!(
                "analytics buffer pool left at the {MAX_POOL_MIB} MiB default: this platform's \
                 memory ceiling could not be read"
            ),
        }
    }
}

/// Which of the two limits bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeilingSource {
    /// Physical RAM.
    PhysicalRam,
    /// A cgroup limit — the systemd unit's `MemoryMax`, or a container's.
    Cgroup,
}

impl CeilingSource {
    /// Words for a log line.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::PhysicalRam => "physical RAM",
            Self::Cgroup => "a cgroup limit, usually the unit's MemoryMax",
        }
    }
}

/// Decide the buffer pool from an operator setting and the detected ceiling.
///
/// `configured` is `BIRDNET_DUCKDB_MEMORY_LIMIT`, honoured verbatim when it is
/// a valid literal: an operator who has measured their own station knows more
/// than this does. `ceiling` is `(MiB, source)`, or `None` when nothing could
/// be read.
#[must_use]
pub fn decide(configured: Option<&str>, ceiling: Option<(u64, CeilingSource)>) -> Budget {
    if let Some(v) = configured.map(str::trim).filter(|v| valid_limit(v)) {
        return Budget::Configured {
            limit: v.to_owned(),
        };
    }
    let Some((ceiling_mib, source)) = ceiling else {
        return Budget::Undetected;
    };
    let share = ceiling_mib / POOL_SHARE_DENOMINATOR;
    if share < MIN_POOL_MIB {
        return Budget::TooSmall {
            ceiling_mib,
            source,
        };
    }
    Budget::Sized {
        pool_mib: share.min(MAX_POOL_MIB),
        ceiling_mib,
        source,
    }
}

/// Whether `v` is a safe `DuckDB` memory-limit literal: a leading ASCII digit
/// followed only by alphanumerics, `.`, or `%` (e.g. `512MB`, `2GB`, `80%`,
/// `1073741824`).
#[must_use]
pub fn valid_limit(v: &str) -> bool {
    v.bytes().next().is_some_and(|b| b.is_ascii_digit())
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'%')
}

/// The smaller of physical RAM and this process's cgroup limit, in MiB.
///
/// `None` on a platform where neither can be read. Linux only, deliberately:
/// the files are read directly rather than through a system-information crate,
/// because this crate has no such dependency and adding one to learn two
/// numbers would be the larger change.
#[must_use]
pub fn detect_ceiling() -> Option<(u64, CeilingSource)> {
    let physical = physical_ram_mib();
    let cgroup = cgroup_limit_mib();
    match (physical, cgroup) {
        (Some(p), Some(c)) if c < p => Some((c, CeilingSource::Cgroup)),
        (Some(p), _) => Some((p, CeilingSource::PhysicalRam)),
        (None, Some(c)) => Some((c, CeilingSource::Cgroup)),
        (None, None) => None,
    }
}

/// `MemTotal` from `/proc/meminfo`, in MiB.
fn physical_ram_mib() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    // "MemTotal:       16316420 kB"
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib / 1024)
}

/// This process's cgroup memory ceiling, in MiB, if it has one.
///
/// cgroup v2 first (`memory.max`, where `max` means unlimited), then v1
/// (`memory.limit_in_bytes`, where "unlimited" is a number close to `u64::MAX`
/// and is filtered by the sanity bound rather than by matching a magic value,
/// which has differed between kernels).
fn cgroup_limit_mib() -> Option<u64> {
    /// Above this a "limit" is the kernel's way of saying unlimited.
    const IMPLAUSIBLE_MIB: u64 = 1024 * 1024; // 1 TiB

    let read = |path: &str| -> Option<u64> {
        let raw = std::fs::read_to_string(path).ok()?;
        let raw = raw.trim();
        if raw == "max" {
            return None;
        }
        let bytes: u64 = raw.parse().ok()?;
        let mib = bytes / (1024 * 1024);
        (mib > 0 && mib < IMPLAUSIBLE_MIB).then_some(mib)
    };
    read("/sys/fs/cgroup/memory.max")
        .or_else(|| read("/sys/fs/cgroup/memory/memory.limit_in_bytes"))
}

#[cfg(test)]
mod tests {
    use super::{
        Budget, CeilingSource, MAX_POOL_MIB, MIN_POOL_MIB, decide, detect_ceiling, valid_limit,
    };

    /// The anchor the whole sizing rests on: a station running the shipped
    /// systemd unit gets exactly the pool it gets today.
    ///
    /// `MemoryMax=1G` and a `256MB` pool is where the "a quarter" comes from.
    ///
    /// Note what this does *not* cover: at a 1 GiB ceiling the cap alone
    /// produces 256 MiB, so halving the fraction still passes here. The
    /// derivation itself is a `const` assertion above, which a changed fraction
    /// fails to compile against.
    #[test]
    fn the_shipped_units_budget_reproduces_todays_default() {
        let budget = decide(None, Some((1024, CeilingSource::Cgroup)));
        assert_eq!(budget.limit().as_deref(), Some("256MB"));
        assert!(matches!(budget, Budget::Sized { pool_mib: 256, .. }));
    }

    /// The case this exists for: a small board no longer hands half its RAM to
    /// one subsystem.
    #[test]
    fn a_small_board_gets_a_small_pool_instead_of_half_its_ram() {
        let budget = decide(None, Some((512, CeilingSource::PhysicalRam)));
        assert_eq!(
            budget.limit().as_deref(),
            Some("128MB"),
            "a 512 MB board used to be told DuckDB could take 256 MB — half of everything \
             it has"
        );
    }

    /// Below the measured floor, analytics is refused rather than started and
    /// killed mid-query.
    #[test]
    fn a_machine_that_cannot_carry_the_floor_is_refused() {
        // A quarter of 200 MiB is 50, under the 64 MiB floor.
        let budget = decide(None, Some((200, CeilingSource::PhysicalRam)));
        assert!(matches!(budget, Budget::TooSmall { .. }), "{budget:?}");
        assert_eq!(budget.limit(), None);
        assert!(
            budget.explain().contains("analytics is off"),
            "the explanation has to say what happened: {}",
            budget.explain()
        );

        // The counterpart, one step up: a quarter of 256 MiB is exactly the
        // floor and is allowed, so the refusal is about the boundary and not
        // about refusing everything small.
        let ok = decide(None, Some((MIN_POOL_MIB * 4, CeilingSource::PhysicalRam)));
        assert_eq!(ok.limit().as_deref(), Some("64MB"));
    }

    /// A big machine does not get a big pool by accident.
    ///
    /// The cap is the flat default this replaces: an operator who wants more
    /// says so, and gets exactly what they said.
    #[test]
    fn a_large_machine_is_capped_at_the_old_default() {
        let budget = decide(None, Some((64 * 1024, CeilingSource::PhysicalRam)));
        assert_eq!(
            budget.limit().as_deref(),
            Some(&format!("{MAX_POOL_MIB}MB")[..])
        );
    }

    /// An operator's own limit wins over the sizing, whatever the machine.
    #[test]
    fn a_configured_limit_is_used_verbatim() {
        let budget = decide(Some("2GB"), Some((512, CeilingSource::PhysicalRam)));
        assert_eq!(budget.limit().as_deref(), Some("2GB"));
        assert!(matches!(budget, Budget::Configured { .. }));

        // Including on a machine the sizing would have refused: someone who has
        // measured their own station knows more than this does.
        let budget = decide(Some("64MB"), Some((100, CeilingSource::PhysicalRam)));
        assert_eq!(budget.limit().as_deref(), Some("64MB"));
    }

    /// Anything that could break out of the `SET memory_limit='…'` statement is
    /// not a limit.
    ///
    /// The sizing must not become a way past the validation the flat default
    /// already had.
    #[test]
    fn a_hostile_limit_is_not_configured() {
        for hostile in [
            "256MB';DROP TABLE detections;--",
            "MB",
            "",
            " ",
            "1 GB",
            "256MB;",
        ] {
            assert!(
                !valid_limit(hostile.trim()),
                "{hostile:?} passed validation"
            );
            let budget = decide(Some(hostile), Some((1024, CeilingSource::Cgroup)));
            assert!(
                !matches!(budget, Budget::Configured { .. }),
                "{hostile:?} was taken as configured"
            );
        }
        // The counterpart, so the test is about hostility and not about
        // rejecting everything.
        for good in ["512MB", "2GB", "80%", "1073741824"] {
            assert!(valid_limit(good), "{good} was rejected");
        }
    }

    /// With nothing detectable the flat default stands, unchanged.
    ///
    /// A platform whose memory cannot be read is one where guessing would be
    /// worse than keeping the behaviour that has shipped for every release.
    #[test]
    fn an_undetectable_machine_keeps_the_flat_default() {
        let budget = decide(None, None);
        assert!(matches!(budget, Budget::Undetected));
        assert_eq!(
            budget.limit().as_deref(),
            Some(&format!("{MAX_POOL_MIB}MB")[..])
        );
    }

    /// The smaller of the two ceilings binds.
    #[test]
    fn the_tighter_of_ram_and_the_cgroup_is_the_ceiling() {
        // Sized from whichever was handed in; the *selection* is
        // `detect_ceiling`'s and is exercised below against this machine.
        let from_cgroup = decide(None, Some((1024, CeilingSource::Cgroup)));
        let from_ram = decide(None, Some((1024, CeilingSource::PhysicalRam)));
        assert_eq!(from_cgroup.limit(), from_ram.limit());
        assert!(from_cgroup.explain().contains("cgroup"));
        assert!(from_ram.explain().contains("physical RAM"));
    }

    /// The detection reads *something* on this machine, and something
    /// plausible.
    ///
    /// Deliberately weak: the value depends on the machine the tests run on, so
    /// asserting a number would be asserting the CI runner's size. What it does
    /// catch is a parse that silently returns zero or a nonsense figure, which
    /// would make every station either refuse analytics or hand DuckDB the
    /// whole box.
    #[test]
    fn the_ceiling_detected_here_is_plausible() {
        let Some((mib, _)) = detect_ceiling() else {
            // A platform with neither file. Nothing to assert, and `decide`
            // already has `an_undetectable_machine_keeps_the_flat_default`.
            return;
        };
        assert!(
            mib >= 64,
            "detected a {mib} MiB machine, which cannot be right"
        );
        assert!(
            mib < 1024 * 1024,
            "detected a {mib} MiB machine — over a terabyte, so a unit is wrong somewhere"
        );
    }
}
