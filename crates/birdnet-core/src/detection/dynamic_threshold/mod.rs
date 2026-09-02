//! Let a species that is *known present* be easier to hear.
//!
//! # The two questions one threshold is answering
//!
//! A fixed confidence threshold is a blunt instrument because it settles two
//! different questions with one number:
//!
//! 1. **Is this a bird at all**, or is it a car door, a gate, wind in a
//!    microphone port?
//! 2. **Is this bird plausible here**, or is it a species whose nearest
//!    population is a continent away?
//!
//! Those want different answers, and once a Tawny Owl has been recorded in the
//! wood at 0.9, a later 0.45 Tawny Owl is very probably another Tawny Owl. A
//! 0.45 for a species never recorded within 500 km is not. Raising the global
//! threshold to suppress the second suppresses the first as well, and every
//! quiet, distant, or partly-masked call of a bird the station already knows is
//! there goes with it.
//!
//! This module lowers the threshold for species the station has *confirmed*,
//! and only those.
//!
//! # How a species is confirmed
//!
//! By a detection at or above [`DynamicThresholdConfig::trigger`] that survived
//! **every other gate** — the privacy filter, the noise-class filter, the
//! occurrence (geomodel) filter, the plausible-hour filter, the corroboration
//! filter. That restriction is the whole safety argument, and it is easy to get
//! wrong: teaching from a raw model output would let one confident false
//! positive make more false positives easier, which is the opposite of what
//! this is for. See [`DynamicThresholds::observe`].
//!
//! Each confirmation raises the species one level, up to
//! [`MAX_LEVEL`], and each level multiplies the base threshold:
//!
//! | Level | Multiplier | A 0.70 base becomes |
//! |---|---|---|
//! | 0 | 1.00 | 0.70 |
//! | 1 | 0.75 | 0.525 |
//! | 2 | 0.50 | 0.35 |
//! | 3 | 0.25 | 0.175 |
//!
//! floored at [`DynamicThresholdConfig::min`], which is a hard floor and not a
//! suggestion.
//!
//! # Why one song cannot reach level 3
//!
//! A blackbird singing for four seconds produces the same species in five
//! overlapping windows. Without a guard, that one song would advance the
//! species three levels in under a second — a station would reach the floor on
//! its first bird and stay there. [`LEARN_COOLDOWN_MS`] is the guard: a species
//! can advance at most once per cooldown, so reaching the floor takes three
//! separate confirmations spread across the day rather than one bird.
//!
//! # It expires
//!
//! A level lasts [`DynamicThresholdConfig::valid_hours`] from its last
//! confirmation. A bird that passed through on migration should not still be
//! making its own species easier to detect a month later. Expiry is evaluated
//! **on read** rather than only by a sweep, so a level that lapsed at 03:00 is
//! already gone to a query at 03:01 whether or not anything swept.
//!
//! # What has to change elsewhere for this to do anything
//!
//! The classifier applies its own threshold before the pipeline ever sees a
//! detection. Lowering a threshold below that gate reaches nothing — the
//! detection was discarded inside the model. So a station running dynamic
//! thresholds must run the model at the floor and let this decide, which
//! [`DynamicThresholdConfig::model_floor`] computes. The extra cost is in
//! post-processing, not inference: the same number of windows are classified,
//! and more candidates survive to be filtered.

use std::collections::HashMap;

/// Highest level a species can reach.
pub const MAX_LEVEL: u8 = 3;

/// Minimum time between two level advances for one species.
///
/// Fifteen minutes. Long enough that a single singing bout — even a nightjar
/// churring for ten minutes — counts once, short enough that a species genuinely
/// present all day reaches the floor within a morning.
pub const LEARN_COOLDOWN_MS: i64 = 15 * 60 * 1000;

/// The multiplier applied to the base threshold at each level.
///
/// Index is the level. Level 0 is 1.0 by construction, which is what makes a
/// species with no learned level indistinguishable from one that has expired.
const LEVEL_MULTIPLIERS: [f32; (MAX_LEVEL as usize) + 1] = [1.0, 0.75, 0.50, 0.25];

/// Configuration for the dynamic threshold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicThresholdConfig {
    /// Whether any adjustment happens at all. Off by default.
    pub enabled: bool,
    /// Confidence at or above which a surviving detection confirms its species.
    ///
    /// Should sit comfortably above the global threshold: the point is that
    /// the station is *sure*, not that it merely accepted the detection.
    pub trigger: f32,
    /// Hard floor for the adjusted threshold.
    ///
    /// No level can take a species below this, whatever the base and however
    /// many confirmations it has. This is the number that bounds the damage if
    /// the rest of the reasoning here is wrong.
    pub min: f32,
    /// How long a level survives without reinforcement.
    pub valid_hours: u32,
}

impl Default for DynamicThresholdConfig {
    /// Off, with the values a station would get if it switched this on and
    /// changed nothing else.
    ///
    /// `trigger` at 0.80 is well above the 0.70 default global threshold, so a
    /// confirmation means the model was confident rather than merely willing.
    /// `min` at 0.25 is the floor a 0.70 base reaches at level 3 anyway, so the
    /// default floor binds only for a station that has *raised* its global
    /// threshold — which is exactly the station that meant it.
    fn default() -> Self {
        Self {
            enabled: false,
            trigger: 0.80,
            min: 0.25,
            valid_hours: 24,
        }
    }
}

impl DynamicThresholdConfig {
    /// The threshold the **classifier** must run at for this to reach anything.
    ///
    /// The lowest value any species could be adjusted to, which is the floor
    /// itself when enabled, and the caller's own threshold when not. A model
    /// gating above this discards the detections the adjustment exists to
    /// recover, and it does so invisibly — the pipeline sees a species that
    /// simply did not call.
    #[must_use]
    pub const fn model_floor(&self, global_threshold: f32) -> f32 {
        if self.enabled {
            self.min.min(global_threshold)
        } else {
            global_threshold
        }
    }

    /// Whether the configuration can ever change a decision.
    ///
    /// A configuration that is enabled but whose floor is at or above the
    /// global threshold adjusts nothing: every level is clamped back to where
    /// it started. That is a silent misconfiguration of exactly the shape this
    /// project keeps finding, so it is named and checkable rather than left to
    /// be discovered from an unchanged detection count.
    #[must_use]
    pub fn is_effective_at(&self, global_threshold: f32) -> bool {
        self.enabled && self.min < global_threshold
    }
}

/// What the station has learned about one species.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeciesLevel {
    /// Current level, `0..=MAX_LEVEL`.
    pub level: u8,
    /// Confirmations counted since the level was first raised above zero.
    pub confirmations: u32,
    /// Epoch milliseconds at which this level lapses without reinforcement.
    pub expires_at_ms: i64,
    /// Epoch milliseconds of the first confirmation of the current episode.
    pub first_learned_ms: i64,
    /// Epoch milliseconds of the most recent confirmation.
    pub last_confirmed_ms: i64,
}

/// A live adjustment: what to multiply a base threshold by, and how low it
/// may go.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Adjustment {
    /// Multiplier for the base threshold, in `(0, 1]`.
    pub multiplier: f64,
    /// Hard floor the result is clamped up to.
    pub floor: f64,
}

impl Adjustment {
    /// Apply this adjustment to `base`.
    ///
    /// Clamped to the floor from below **and to `base` from above**, so a base
    /// already under the floor — an operator who has deliberately made one
    /// species very easy — is never quietly raised.
    #[must_use]
    pub fn apply(self, base: f64) -> f64 {
        (base * self.multiplier).max(self.floor).min(base)
    }
}

/// The learned state for every species, and the rules that change it.
///
/// Held in memory by the detection event processor and persisted so a restart
/// does not forget what the site contains.
#[derive(Debug, Clone)]
pub struct DynamicThresholds {
    config: DynamicThresholdConfig,
    levels: HashMap<String, SpeciesLevel>,
}

impl DynamicThresholds {
    /// An empty tracker with the given configuration.
    #[must_use]
    pub fn new(config: DynamicThresholdConfig) -> Self {
        Self {
            config,
            levels: HashMap::new(),
        }
    }

    /// The configuration in force.
    #[must_use]
    pub const fn config(&self) -> DynamicThresholdConfig {
        self.config
    }

    /// The adjustment in force for `sci_name` right now, if any.
    ///
    /// `None` when the feature is off, when the species has no level, and when
    /// its level has lapsed — three cases a caller must not be able to tell
    /// apart, because they all mean "nothing learned applies".
    ///
    /// Returned as a multiplier and a floor rather than as a finished
    /// threshold so a caller can apply it in its own precision. That is not
    /// fastidiousness: per-species thresholds come from SQLite `REAL` columns
    /// and are compared in `f64`, and an earlier version of this that took and
    /// returned `f32` turned a stored 0.8 into 0.800000011920929 on the way
    /// through. Two existing tests caught it, both asserting the threshold a
    /// quarantine row records — which an operator reads.
    #[must_use]
    pub fn adjustment(&self, sci_name: &str, now_ms: i64) -> Option<Adjustment> {
        if !self.config.enabled {
            return None;
        }
        let state = self.levels.get(sci_name)?;
        if now_ms >= state.expires_at_ms {
            return None;
        }
        let multiplier = LEVEL_MULTIPLIERS
            .get(state.level as usize)
            .copied()
            .unwrap_or(1.0);
        Some(Adjustment {
            multiplier: f64::from(multiplier),
            floor: f64::from(self.config.min),
        })
    }

    /// The threshold to apply to `sci_name` right now.
    ///
    /// `base` is whatever would apply without this: the species' operator-set
    /// override, or the global threshold. Returns `base` unchanged when
    /// [`Self::adjustment`] returns `None`.
    #[must_use]
    pub fn effective_threshold(&self, sci_name: &str, base: f32, now_ms: i64) -> f32 {
        self.adjustment(sci_name, now_ms).map_or(base, |a| {
            #[allow(clippy::cast_possible_truncation)]
            {
                a.apply(f64::from(base)) as f32
            }
        })
    }

    /// Record a detection that **survived every other gate**.
    ///
    /// Returns `true` when the species' level changed.
    ///
    /// The caller's obligation is the whole safety property: this must be fed
    /// accepted detections, never raw model output. A species the occurrence
    /// filter excluded, a chunk the noise filter dropped, a record quarantined
    /// for an implausible hour — none of those has confirmed anything, and
    /// letting one lower its own species' threshold would compound a false
    /// positive into a stream of them.
    pub fn observe(&mut self, sci_name: &str, confidence: f32, now_ms: i64) -> bool {
        if !self.config.enabled || confidence < self.config.trigger || sci_name.is_empty() {
            return false;
        }
        let valid_ms = i64::from(self.config.valid_hours) * 3_600_000;

        match self.levels.get_mut(sci_name) {
            None => {
                self.levels.insert(
                    sci_name.to_owned(),
                    SpeciesLevel {
                        level: 1,
                        confirmations: 1,
                        expires_at_ms: now_ms.saturating_add(valid_ms),
                        first_learned_ms: now_ms,
                        last_confirmed_ms: now_ms,
                    },
                );
                true
            }
            Some(state) => {
                let lapsed = now_ms >= state.expires_at_ms;
                // A lapsed episode restarts at level 1 rather than resuming
                // where it left off. Resuming would mean a species heard once a
                // month climbed to the floor over a year without ever having
                // been present twice in the same window, which is the opposite
                // of the claim the level is making.
                if lapsed {
                    *state = SpeciesLevel {
                        level: 1,
                        confirmations: 1,
                        expires_at_ms: now_ms.saturating_add(valid_ms),
                        first_learned_ms: now_ms,
                        last_confirmed_ms: now_ms,
                    };
                    return true;
                }

                // Every confirmation extends the lease, whether or not it also
                // advances the level: a species still calling is still present.
                state.expires_at_ms = now_ms.saturating_add(valid_ms);
                state.confirmations = state.confirmations.saturating_add(1);

                let cooled = now_ms.saturating_sub(state.last_confirmed_ms) >= LEARN_COOLDOWN_MS;
                state.last_confirmed_ms = now_ms;
                if cooled && state.level < MAX_LEVEL {
                    state.level += 1;
                    return true;
                }
                false
            }
        }
    }

    /// Drop lapsed entries, returning how many went.
    ///
    /// Purely housekeeping: [`Self::effective_threshold`] already ignores a
    /// lapsed entry, so this only bounds memory and keeps the persisted set
    /// honest. It is not load-bearing, which is deliberate — a sweep that
    /// failed to run must not be able to leave a stale level in force.
    pub fn expire(&mut self, now_ms: i64) -> usize {
        let before = self.levels.len();
        self.levels.retain(|_, s| now_ms < s.expires_at_ms);
        before - self.levels.len()
    }

    /// Every species with a live level, for persistence and for the UI.
    #[must_use]
    pub fn snapshot(&self, now_ms: i64) -> Vec<(String, SpeciesLevel)> {
        let mut out: Vec<(String, SpeciesLevel)> = self
            .levels
            .iter()
            .filter(|(_, s)| now_ms < s.expires_at_ms)
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Reinstate persisted state after a restart.
    ///
    /// Lapsed entries are dropped on the way in rather than being restored and
    /// swept later.
    ///
    /// This is housekeeping, not correctness, and the distinction is worth
    /// stating because the first version of this comment got it wrong. It
    /// claimed the filter stopped a station that had been off for a week from
    /// "briefly honouring a week-old level" — which it does not, because
    /// [`Self::effective_threshold`] checks expiry on every read and a
    /// restored-but-lapsed entry is already inert. A mutant that removed the
    /// filter entirely left all nineteen gates green, which is how that was
    /// found. What it actually buys is a bounded set: a station restarting
    /// after a season would otherwise load every species it had ever confirmed
    /// and carry them until something swept. [`Self::tracked_count`] is what
    /// makes that checkable.
    pub fn restore(&mut self, rows: impl IntoIterator<Item = (String, SpeciesLevel)>, now_ms: i64) {
        for (name, state) in rows {
            if now_ms < state.expires_at_ms {
                self.levels.insert(name, state);
            }
        }
    }

    /// How many species currently carry a live level.
    #[must_use]
    pub fn live_count(&self, now_ms: i64) -> usize {
        self.levels
            .values()
            .filter(|s| now_ms < s.expires_at_ms)
            .count()
    }

    /// How many entries are held, live or lapsed.
    ///
    /// Differs from [`Self::live_count`] only between a level lapsing and
    /// something sweeping it. That gap is invisible to every other method here
    /// — a lapsed entry cannot change a threshold — so this exists to make the
    /// *memory* claim checkable rather than to be consulted by the pipeline.
    #[must_use]
    pub fn tracked_count(&self) -> usize {
        self.levels.len()
    }
}

#[cfg(test)]
mod tests;
