//! The meter: filter bank in, interval statistics out.

use super::bands::{
    CENTRE_FREQUENCIES_HZ, THIRD_OCTAVE_EDGE_RATIO, a_weighting_db, band_label, exact_centre_hz,
};
use super::filter::{BiquadError, ThirdOctaveBand};

/// Lowest level the meter will report, in decibels relative to full scale.
///
/// Digital silence is negative infinity, which is not a number a chart, a
/// database column or an average can carry. −120 dBFS is far below the noise
/// floor of any microphone and preamp a station will have, so clamping there
/// loses nothing real and keeps every downstream consumer arithmetic-safe.
pub const FLOOR_DBFS: f32 = -120.0;

/// Fraction of Nyquist a band's upper edge must stay below.
///
/// A bandpass designed by the bilinear transform warps towards Nyquist: the
/// band gets narrower and its gain wrong as the edge approaches it. Bands that
/// do not fit are dropped rather than reported with a quiet error, so a 22.05
/// kHz station reports 27 bands and a 48 kHz one reports 30, and neither of
/// them reports a band it cannot actually measure.
pub const NYQUIST_MARGIN: f32 = 0.95;

/// Statistics for one third-octave band over one interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandLevel {
    /// Nominal ISO 266 centre frequency, in hertz — the *label*, not the
    /// frequency the filter was designed at. See
    /// [`super::exact_centre_hz`] for why those differ and by how much.
    pub centre_hz: f32,
    /// Quietest one-second level in the interval, in dBFS (plus calibration).
    pub min_db: f32,
    /// Loudest one-second level in the interval.
    pub max_db: f32,
    /// Energy mean of the one-second levels.
    ///
    /// Energy mean, not the arithmetic mean of the decibel values: decibels
    /// are logarithmic, and averaging them gives a number that is not the
    /// level of anything.
    ///
    /// The distinction is not academic. For a band that spends thirty seconds
    /// at −80 dB and one second at −20 dB, the arithmetic mean of the logs is
    /// −78.06 dB and the energy mean is −34.91 dB — a gap of 43 dB. The second
    /// is the level of the interval; the first is the average of two unrelated
    /// numbers, and it hides exactly the transient a soundscape series exists
    /// to show. (Those two figures are computed, not estimated: the first
    /// draft of this comment said −79 and −52, which is what a plausible guess
    /// looks like. `the_interval_mean_is_an_energy_mean` pins both.)
    pub mean_db: f32,
    /// One-second measurements that went into these figures.
    pub seconds: u32,
}

/// One interval's complete measurement.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundLevelReading {
    /// Per-band statistics, in ascending centre-frequency order.
    pub bands: Vec<BandLevel>,
    /// Broadband A-weighted level over the interval, in dB(A).
    ///
    /// The power sum of the per-band energy means with the IEC 61672
    /// A-weighting applied. This is the single number an environmental noise
    /// measurement quotes.
    pub a_weighted_db: f32,
    /// Broadband unweighted (Z-weighted) level over the interval.
    ///
    /// Reported alongside the A-weighted figure because the *difference*
    /// between them is diagnostic: a large gap means the energy is
    /// low-frequency — wind, traffic, a mount resonating — which is the noise
    /// that ruins classification while barely moving the dB(A).
    pub z_weighted_db: f32,
    /// Seconds of audio the interval covers.
    pub interval_secs: u32,
}

/// How the meter turns full-scale digital level into a reported figure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Calibration {
    /// Uncalibrated: report dBFS, which is negative and station-relative.
    ///
    /// Honest default. A dBFS series is perfectly good for the questions a
    /// station actually asks — is it getting louder, is the microphone dying,
    /// when is the dawn chorus — as long as the gain does not change. It is
    /// *not* comparable with another station or with a published figure, and
    /// labelling it "dB SPL" would imply that it is.
    FullScale,
    /// Calibrated: add this many decibels to reach dB SPL.
    ///
    /// The offset is the sound pressure level that produces a full-scale
    /// digital signal on this microphone at this gain, obtained by playing a
    /// known level (a 94 dB SPL calibrator at 1 kHz is the usual one) and
    /// reading what the meter reports uncalibrated.
    SplOffsetDb(f32),
}

impl Calibration {
    /// The offset to add to a dBFS figure.
    #[must_use]
    pub const fn offset_db(self) -> f32 {
        match self {
            Self::FullScale => 0.0,
            Self::SplOffsetDb(db) => db,
        }
    }

    /// The unit these readings are in, for labelling.
    #[must_use]
    pub const fn unit(self) -> &'static str {
        match self {
            Self::FullScale => "dBFS",
            Self::SplOffsetDb(_) => "dB SPL",
        }
    }
}

/// Per-band running state: the filter, the current second, and the interval.
#[derive(Debug, Clone)]
struct BandState {
    /// The rounded label reported to callers.
    centre_hz: f32,
    /// The exact base-10 centre the filter and the weighting use.
    exact_hz: f32,
    filter: ThirdOctaveBand,
    /// Sum of squares of filtered samples in the second being accumulated.
    sum_sq: f64,
    /// Interval statistics, accumulated in linear power so the mean is an
    /// energy mean rather than an average of logarithms.
    min_db: f32,
    max_db: f32,
    power_sum: f64,
    seconds: u32,
}

/// A third-octave sound level meter for one audio source.
///
/// Feed it samples with [`Self::push`]; it returns a [`SoundLevelReading`]
/// each time a whole interval has gone by.
///
/// # Memory
///
/// Constant, and small: one filter and four accumulators per band, about 100
/// bytes each, so under 4 kB for a 30-band meter regardless of sample rate or
/// interval. Nothing is allocated per sample and nothing is buffered — the
/// running sum of squares is all a root-mean-square needs. (The reference
/// implementation this was measured against buffers a whole second of `f64`
/// per band before taking the RMS, which at 48 kHz across 30 bands is about
/// 11 MB of live data on a machine that may have 512 MB in total.)
#[derive(Debug, Clone)]
pub struct SoundLevelMeter {
    sample_rate: u32,
    interval_secs: u32,
    calibration: Calibration,
    bands: Vec<BandState>,
    /// Samples accumulated into the current one-second window.
    samples_this_second: u32,
    /// Whole seconds accumulated into the current interval.
    seconds_this_interval: u32,
}

impl SoundLevelMeter {
    /// Build a meter for `sample_rate`, aggregating over `interval_secs`.
    ///
    /// Bands whose upper edge reaches [`NYQUIST_MARGIN`] of Nyquist are
    /// omitted; [`Self::band_count`] reports how many survived.
    ///
    /// # Errors
    ///
    /// [`BiquadError`] if a band that passed the Nyquist check still produced
    /// unstable coefficients. That should not happen for any supported rate
    /// and is surfaced rather than skipped, because a silently missing band is
    /// a hole in a series that nothing downstream can distinguish from silence.
    pub fn new(
        sample_rate: u32,
        interval_secs: u32,
        calibration: Calibration,
    ) -> Result<Self, BiquadError> {
        if sample_rate == 0 {
            return Err(BiquadError::NonPositiveParameter);
        }
        let interval_secs = interval_secs.max(1);
        #[allow(clippy::cast_precision_loss)]
        let nyquist = sample_rate as f32 / 2.0;
        let ceiling = nyquist * NYQUIST_MARGIN;

        let mut bands = Vec::with_capacity(CENTRE_FREQUENCIES_HZ.len());
        for (index, centre_hz) in CENTRE_FREQUENCIES_HZ.into_iter().enumerate() {
            let exact_hz = exact_centre_hz(index);
            if exact_hz * THIRD_OCTAVE_EDGE_RATIO >= ceiling {
                continue;
            }
            bands.push(BandState {
                centre_hz,
                exact_hz,
                filter: ThirdOctaveBand::new(exact_hz, sample_rate)?,
                sum_sq: 0.0,
                min_db: f32::INFINITY,
                max_db: f32::NEG_INFINITY,
                power_sum: 0.0,
                seconds: 0,
            });
        }

        Ok(Self {
            sample_rate,
            interval_secs,
            calibration,
            bands,
            samples_this_second: 0,
            seconds_this_interval: 0,
        })
    }

    /// How many bands this meter measures at its sample rate.
    #[must_use]
    pub const fn band_count(&self) -> usize {
        self.bands.len()
    }

    /// The centre frequencies this meter measures, ascending.
    #[must_use]
    pub fn centre_frequencies(&self) -> Vec<f32> {
        self.bands.iter().map(|b| b.centre_hz).collect()
    }

    /// The calibration in force.
    #[must_use]
    pub const fn calibration(&self) -> Calibration {
        self.calibration
    }

    /// Feed samples in `[-1.0, 1.0]`, returning a reading once a whole
    /// interval has accumulated.
    ///
    /// Non-finite samples are treated as silence rather than propagated: one
    /// NaN in a decode would otherwise poison every band's running sum for the
    /// rest of the interval, and every subsequent interval through the filter
    /// state.
    pub fn push(&mut self, samples: &[f32]) -> Option<SoundLevelReading> {
        let mut reading = None;
        for &sample in samples {
            let sample = if sample.is_finite() { sample } else { 0.0 };
            for band in &mut self.bands {
                let y = band.filter.process(sample);
                if y.is_finite() {
                    band.sum_sq += y * y;
                } else {
                    band.filter.reset();
                }
            }
            self.samples_this_second += 1;
            if self.samples_this_second >= self.sample_rate {
                self.close_second();
                if self.seconds_this_interval >= self.interval_secs {
                    reading = Some(self.close_interval());
                }
            }
        }
        reading
    }

    /// Fold the finished second into each band's interval statistics.
    fn close_second(&mut self) {
        let n = f64::from(self.samples_this_second);
        for band in &mut self.bands {
            let mean_sq = if n > 0.0 { band.sum_sq / n } else { 0.0 };
            let db = power_to_db(mean_sq);
            band.min_db = band.min_db.min(db);
            band.max_db = band.max_db.max(db);
            // Accumulated in linear power, so the interval mean is an energy
            // mean. Clamped through `db_to_power(power_to_db(..))` rather than
            // summed raw so that a floored second contributes the floor's
            // energy and not zero.
            band.power_sum += db_to_power(db);
            band.seconds += 1;
            band.sum_sq = 0.0;
        }
        self.samples_this_second = 0;
        self.seconds_this_interval += 1;
    }

    /// Emit the interval and reset for the next one.
    fn close_interval(&mut self) -> SoundLevelReading {
        let offset = self.calibration.offset_db();
        let mut bands = Vec::with_capacity(self.bands.len());
        let mut a_power = 0.0_f64;
        let mut z_power = 0.0_f64;

        for band in &mut self.bands {
            let seconds = band.seconds.max(1);
            let mean_db = power_to_db(band.power_sum / f64::from(seconds));
            let a_offset = a_weighting_db(band.exact_hz);
            a_power += db_to_power(mean_db + a_offset);
            z_power += db_to_power(mean_db);

            bands.push(BandLevel {
                centre_hz: band.centre_hz,
                min_db: band.min_db + offset,
                max_db: band.max_db + offset,
                mean_db: mean_db + offset,
                seconds: band.seconds,
            });

            band.min_db = f32::INFINITY;
            band.max_db = f32::NEG_INFINITY;
            band.power_sum = 0.0;
            band.seconds = 0;
        }

        let interval_secs = self.seconds_this_interval;
        self.seconds_this_interval = 0;

        SoundLevelReading {
            bands,
            a_weighted_db: power_to_db(a_power) + offset,
            z_weighted_db: power_to_db(z_power) + offset,
            interval_secs,
        }
    }

    /// Discard all accumulated state, keeping the configuration.
    ///
    /// Call this across a capture discontinuity — a source restart, a device
    /// change — so the filters' ringing from the old signal does not land in
    /// the first second of the new one.
    pub fn reset(&mut self) {
        for band in &mut self.bands {
            band.filter.reset();
            band.sum_sq = 0.0;
            band.min_db = f32::INFINITY;
            band.max_db = f32::NEG_INFINITY;
            band.power_sum = 0.0;
            band.seconds = 0;
        }
        self.samples_this_second = 0;
        self.seconds_this_interval = 0;
    }

    /// Whether any band's filter state has gone non-finite.
    #[must_use]
    pub fn is_diverged(&self) -> bool {
        self.bands.iter().any(|b| b.filter.is_diverged())
    }
}

/// Mean square to decibels, floored at [`FLOOR_DBFS`].
#[must_use]
pub fn power_to_db(mean_square: f64) -> f32 {
    if !(mean_square.is_finite()) || mean_square <= 0.0 {
        return FLOOR_DBFS;
    }
    #[allow(clippy::cast_possible_truncation)]
    let db = (10.0 * mean_square.log10()) as f32;
    if db.is_finite() {
        db.max(FLOOR_DBFS)
    } else {
        FLOOR_DBFS
    }
}

/// Decibels back to mean square.
#[must_use]
pub fn db_to_power(db: f32) -> f64 {
    10.0_f64.powf(f64::from(db) / 10.0)
}

/// The label for a band, re-exported so callers do not reach into `bands`.
#[must_use]
pub fn label_for(centre_hz: f32) -> String {
    band_label(centre_hz)
}
