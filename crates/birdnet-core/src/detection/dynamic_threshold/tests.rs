//! Gates for the dynamic threshold.

use super::*;

const HOUR_MS: i64 = 3_600_000;

fn on() -> DynamicThresholdConfig {
    DynamicThresholdConfig {
        enabled: true,
        trigger: 0.80,
        min: 0.20,
        valid_hours: 24,
    }
}

// ---------------------------------------------------------------------------
// The adjustment itself
// ---------------------------------------------------------------------------

/// A species with no history gets the base threshold, unchanged.
#[test]
fn an_unheard_species_is_not_adjusted() {
    let d = DynamicThresholds::new(on());
    assert!((d.effective_threshold("Strix aluco", 0.70, 0) - 0.70).abs() < 1e-6);
}

/// One confirmation buys one level: 0.70 becomes 0.525.
#[test]
fn one_confirmation_lowers_the_threshold_one_level() {
    let mut d = DynamicThresholds::new(on());
    assert!(d.observe("Strix aluco", 0.9, 0), "level should change");
    let t = d.effective_threshold("Strix aluco", 0.70, 1000);
    assert!(
        (t - 0.525).abs() < 1e-4,
        "expected 0.70 * 0.75 = 0.525, got {t}"
    );
}

/// And only for that species. The counterpart: an adjustment that applied to
/// everything would pass every other test in this file.
#[test]
fn the_adjustment_is_per_species() {
    let mut d = DynamicThresholds::new(on());
    d.observe("Strix aluco", 0.9, 0);
    assert!(
        (d.effective_threshold("Turdus merula", 0.70, 1000) - 0.70).abs() < 1e-6,
        "a blackbird must not benefit from an owl being present"
    );
}

/// Three spaced confirmations reach the maximum level, and a fourth does not
/// go past it.
#[test]
fn three_spaced_confirmations_reach_the_floor_and_stop() {
    let mut d = DynamicThresholds::new(on());
    let mut t = 0;
    for _ in 0..5 {
        d.observe("Strix aluco", 0.9, t);
        t += LEARN_COOLDOWN_MS;
    }
    let level = d.snapshot(t)[0].1.level;
    assert_eq!(
        level, MAX_LEVEL,
        "five confirmations should cap at MAX_LEVEL"
    );
    let threshold = d.effective_threshold("Strix aluco", 0.70, t);
    assert!(
        (threshold - 0.20).abs() < 1e-4,
        "0.70 * 0.25 = 0.175, below the 0.20 floor, so the floor should bind; got {threshold}"
    );
}

/// The floor is a floor. A base low enough that level 3 would go under it is
/// clamped, not honoured.
#[test]
fn the_floor_binds() {
    let mut d = DynamicThresholds::new(DynamicThresholdConfig { min: 0.40, ..on() });
    let mut t = 0;
    for _ in 0..4 {
        d.observe("Strix aluco", 0.9, t);
        t += LEARN_COOLDOWN_MS;
    }
    let threshold = d.effective_threshold("Strix aluco", 0.70, t);
    assert!(
        (threshold - 0.40).abs() < 1e-6,
        "the floor of 0.40 must bind against 0.70 * 0.25 = 0.175, got {threshold}"
    );
}

/// The adjustment never *raises* a threshold, even if a base is already below
/// the floor.
///
/// A species whose operator-set override is 0.10 has been deliberately made
/// easy; a floor of 0.20 must not quietly undo that. Clamping to the floor
/// without also clamping to the base would do exactly that.
#[test]
fn a_base_below_the_floor_is_never_raised() {
    let mut d = DynamicThresholds::new(on());
    d.observe("Strix aluco", 0.9, 0);
    let t = d.effective_threshold("Strix aluco", 0.10, 1000);
    assert!(
        (t - 0.10).abs() < 1e-6,
        "a base of 0.10 must stay 0.10, got {t}"
    );
}

// ---------------------------------------------------------------------------
// What may teach, and how often
// ---------------------------------------------------------------------------

/// A detection below the trigger confirms nothing, however many arrive.
#[test]
fn a_detection_below_the_trigger_teaches_nothing() {
    let mut d = DynamicThresholds::new(on());
    for i in 0..10 {
        assert!(!d.observe("Strix aluco", 0.79, i * LEARN_COOLDOWN_MS));
    }
    assert!((d.effective_threshold("Strix aluco", 0.70, 0) - 0.70).abs() < 1e-6);
    assert_eq!(d.live_count(0), 0);
}

/// One singing bout is one confirmation.
///
/// Five overlapping windows of the same blackbird arrive within a second. That
/// is one bird, and without the cooldown it would take the species from
/// nothing to the floor before the song finished.
#[test]
fn one_singing_bout_advances_at_most_one_level() {
    let mut d = DynamicThresholds::new(on());
    for i in 0..5 {
        d.observe("Turdus merula", 0.95, i * 200);
    }
    let state = d.snapshot(1000)[0].1;
    assert_eq!(
        state.level, 1,
        "five windows of one song advanced the level to {} — the cooldown is not holding",
        state.level
    );
    assert_eq!(
        state.confirmations, 5,
        "every window should still be counted, even though only one advanced the level"
    );
}

/// And the counterpart: confirmations that *are* spread out do advance.
///
/// Without this, a cooldown that never expired would satisfy the test above
/// perfectly and the feature would be inert.
#[test]
fn confirmations_spread_beyond_the_cooldown_do_advance() {
    let mut d = DynamicThresholds::new(on());
    d.observe("Turdus merula", 0.95, 0);
    d.observe("Turdus merula", 0.95, LEARN_COOLDOWN_MS);
    assert_eq!(d.snapshot(LEARN_COOLDOWN_MS)[0].1.level, 2);
}

// ---------------------------------------------------------------------------
// Expiry
// ---------------------------------------------------------------------------

/// A level lapses, and lapses on *read* rather than waiting for a sweep.
#[test]
fn a_level_lapses_without_being_swept() {
    let mut d = DynamicThresholds::new(on());
    d.observe("Strix aluco", 0.9, 0);

    let inside = 23 * HOUR_MS;
    let outside = 25 * HOUR_MS;
    assert!(
        d.effective_threshold("Strix aluco", 0.70, inside) < 0.70,
        "still inside the 24-hour lease"
    );
    assert!(
        (d.effective_threshold("Strix aluco", 0.70, outside) - 0.70).abs() < 1e-6,
        "past the lease the base must apply, whether or not anything swept"
    );
}

/// Each confirmation extends the lease.
#[test]
fn a_confirmation_extends_the_lease() {
    let mut d = DynamicThresholds::new(on());
    d.observe("Strix aluco", 0.9, 0);
    d.observe("Strix aluco", 0.9, 20 * HOUR_MS);
    assert!(
        d.effective_threshold("Strix aluco", 0.70, 40 * HOUR_MS) < 0.70,
        "the second confirmation should have carried the lease past 40 hours"
    );
}

/// A lapsed episode restarts at level 1, it does not resume.
///
/// Resuming would let a species heard once a month climb to the floor over a
/// year without ever having been present twice in one window — the level would
/// stop meaning "confirmed present" and start meaning "seen occasionally".
#[test]
fn a_lapsed_episode_restarts_rather_than_resuming() {
    let mut d = DynamicThresholds::new(on());
    d.observe("Strix aluco", 0.9, 0);
    d.observe("Strix aluco", 0.9, LEARN_COOLDOWN_MS);
    assert_eq!(d.snapshot(LEARN_COOLDOWN_MS)[0].1.level, 2);

    let after_lapse = 48 * HOUR_MS;
    d.observe("Strix aluco", 0.9, after_lapse);
    let state = d.snapshot(after_lapse)[0].1;
    assert_eq!(state.level, 1, "a lapsed episode must restart at level 1");
    assert_eq!(state.confirmations, 1, "and its count restarts too");
    assert_eq!(state.first_learned_ms, after_lapse);
}

/// `expire` removes lapsed entries and keeps live ones.
#[test]
fn expire_removes_only_what_has_lapsed() {
    let mut d = DynamicThresholds::new(on());
    d.observe("Strix aluco", 0.9, 0);
    d.observe("Turdus merula", 0.9, 20 * HOUR_MS);

    assert_eq!(d.expire(25 * HOUR_MS), 1, "only the owl has lapsed");
    let live = d.snapshot(25 * HOUR_MS);
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].0, "Turdus merula");
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

/// A snapshot restores to the same effective thresholds.
#[test]
fn a_snapshot_round_trips() {
    let mut d = DynamicThresholds::new(on());
    d.observe("Strix aluco", 0.9, 0);
    d.observe("Strix aluco", 0.9, LEARN_COOLDOWN_MS);
    let saved = d.snapshot(LEARN_COOLDOWN_MS);

    let mut restored = DynamicThresholds::new(on());
    restored.restore(saved, LEARN_COOLDOWN_MS);
    assert!(
        (restored.effective_threshold("Strix aluco", 0.70, LEARN_COOLDOWN_MS)
            - d.effective_threshold("Strix aluco", 0.70, LEARN_COOLDOWN_MS))
        .abs()
            < 1e-6
    );
}

/// A station that was off long enough comes back holding nothing.
///
/// The behavioural half of this — that a stale level cannot change a
/// threshold — is guaranteed by read-time expiry and would pass with no filter
/// in `restore` at all; a mutant that removed the filter left every other gate
/// green. So the assertion that discriminates is on what is *held*, not on
/// what it does: without the filter a station restarting after a season loads
/// every species it ever confirmed and carries them until something sweeps.
#[test]
fn restoring_after_a_long_outage_holds_nothing() {
    let mut d = DynamicThresholds::new(on());
    d.observe("Strix aluco", 0.9, 0);
    let saved = d.snapshot(0);
    assert_eq!(
        saved.len(),
        1,
        "the snapshot must carry the entry to restore"
    );

    let mut restored = DynamicThresholds::new(on());
    restored.restore(saved, 7 * 24 * HOUR_MS);
    assert_eq!(
        restored.tracked_count(),
        0,
        "a lapsed entry must not be loaded at all, not merely ignored once loaded"
    );
    assert_eq!(restored.live_count(7 * 24 * HOUR_MS), 0);
    assert!(
        (restored.effective_threshold("Strix aluco", 0.70, 7 * 24 * HOUR_MS) - 0.70).abs() < 1e-6
    );
}

/// And the counterpart: a live entry *is* loaded.
///
/// Without this, a `restore` that discarded everything would satisfy the test
/// above perfectly, and a restart would silently forget what the site
/// contains — which is the whole reason the state is persisted.
#[test]
fn restoring_within_the_lease_holds_the_entry() {
    let mut d = DynamicThresholds::new(on());
    d.observe("Strix aluco", 0.9, 0);
    let saved = d.snapshot(0);

    let mut restored = DynamicThresholds::new(on());
    restored.restore(saved, HOUR_MS);
    assert_eq!(restored.tracked_count(), 1);
    assert!(restored.effective_threshold("Strix aluco", 0.70, HOUR_MS) < 0.70);
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Disabled means nothing happens at all — not even bookkeeping.
#[test]
fn disabled_changes_nothing() {
    let mut d = DynamicThresholds::new(DynamicThresholdConfig::default());
    assert!(!d.observe("Strix aluco", 0.99, 0));
    assert_eq!(d.live_count(0), 0);
    assert!((d.effective_threshold("Strix aluco", 0.70, 0) - 0.70).abs() < 1e-6);
}

/// The model must be told to run at the floor, or none of this reaches
/// anything.
///
/// The classifier applies its own threshold before the pipeline sees a
/// detection. A station that enabled dynamic thresholds and left the model
/// gating at 0.70 would see no change at all and no error — the detections the
/// adjustment exists to recover were discarded inside the model.
#[test]
fn the_model_floor_is_the_lowest_any_species_could_reach() {
    let cfg = on();
    assert!(
        (cfg.model_floor(0.70) - 0.20).abs() < 1e-6,
        "enabled, the model must gate at the configured floor"
    );
    assert!(
        (DynamicThresholdConfig::default().model_floor(0.70) - 0.70).abs() < 1e-6,
        "disabled, the model must gate where the operator set it"
    );
    // And the floor never *raises* the model's gate: a station with a global
    // threshold below the floor keeps its own.
    assert!((cfg.model_floor(0.10) - 0.10).abs() < 1e-6);
}

/// A configuration that cannot change any decision says so.
///
/// Enabled with a floor at or above the global threshold adjusts nothing:
/// every level clamps straight back. That is a configured-and-inert state, the
/// exact shape this project keeps finding, so it is checkable rather than
/// discoverable from an unchanged detection count.
#[test]
fn an_inert_configuration_is_reported_as_inert() {
    let cfg = DynamicThresholdConfig { min: 0.70, ..on() };
    assert!(
        !cfg.is_effective_at(0.70),
        "floor == global adjusts nothing"
    );
    assert!(
        !cfg.is_effective_at(0.60),
        "floor above global adjusts nothing"
    );
    assert!(cfg.is_effective_at(0.80), "floor below global does adjust");
    assert!(
        !DynamicThresholdConfig::default().is_effective_at(0.70),
        "disabled is inert whatever the numbers"
    );

    // ...and the claim is true of the behaviour, not just the predicate.
    let mut d = DynamicThresholds::new(cfg);
    d.observe("Strix aluco", 0.9, 0);
    assert!(
        (d.effective_threshold("Strix aluco", 0.70, 1000) - 0.70).abs() < 1e-6,
        "is_effective_at said this configuration is inert; the threshold must be unchanged"
    );
}

/// The adjustment is exposed as a multiplier and floor, and applying it in
/// `f64` does not perturb the value.
///
/// The `f32` round-trip this replaced turned a stored per-species threshold of
/// 0.8 into 0.800000011920929, which reached the quarantine row an operator
/// reads. Two pre-existing tests in the binary caught it; this pins the
/// property where the arithmetic lives.
#[test]
fn an_unadjusted_threshold_passes_through_bit_exact() {
    let mut d = DynamicThresholds::new(on());
    assert!(
        d.adjustment("Strix aluco", 0).is_none(),
        "an unheard species has no adjustment at all, not an identity one"
    );

    d.observe("Strix aluco", 0.9, 0);
    let a = d.adjustment("Strix aluco", 1000).expect("adjustment");
    assert!(
        (a.multiplier - 0.75).abs() < 1e-12,
        "the multipliers are exact powers of two"
    );
    // The floor is a configured `f32`, so widening it lands 3e-9 away from
    // 0.20 and always will. That is immaterial for a floor — nobody sets one to
    // nine decimal places — and it is *not* the value the precision argument is
    // about, which is the per-species threshold read from SQLite.
    assert!((a.floor - 0.20).abs() < 1e-7, "got {}", a.floor);
    // 0.75 is exact and 0.8 is the nearest double to 0.8, so applying the
    // adjustment must not add error beyond that one multiplication.
    assert!((a.apply(0.8) - 0.6).abs() < 1e-12, "got {}", a.apply(0.8));
}

/// An empty species name is not a species.
#[test]
fn an_empty_species_name_is_ignored() {
    let mut d = DynamicThresholds::new(on());
    assert!(!d.observe("", 0.99, 0));
    assert_eq!(d.live_count(0), 0);
}
