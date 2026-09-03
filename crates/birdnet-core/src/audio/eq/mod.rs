//! A configurable filter chain for conditioning one capture source.
//!
//! # What this replaces
//!
//! Three booleans. [`crate::audio::capture::AudioPipeline`] offered a
//! high-pass at a fixed 120 Hz, a DC block at a fixed 5 Hz, and an automatic
//! gain control — honest, well documented, and a compromise chosen for a
//! garden.
//!
//! Sites differ in the noise they are fighting, and the fixed corner is wrong
//! for most of them in a different direction:
//!
//! * A station beside a motorway needs a steeper low cut than one pole at
//!   120 Hz. Two or three passes of the same section is the difference between
//!   attenuating traffic and merely tilting it.
//! * A station under a fluorescent transformer, or on a long unbalanced cable,
//!   has mains hum at 50 or 60 Hz **and its harmonics**. No high-pass removes
//!   a 150 Hz harmonic without also removing the bitterns and wood pigeons
//!   around it; a notch removes the hum and nothing else.
//! * A hydrophone, a nest-box microphone, or a bat detector wants the band
//!   moved somewhere else entirely.
//!
//! # Nothing changes for a station that does not ask
//!
//! [`EqChain::from_pipeline_flags`] reproduces the three booleans exactly, and
//! that is what a station carries until an operator edits it. An upgrade must
//! not change what a microphone sounds like.
//!
//! # Two backends, one chain
//!
//! Audio reaches the station two ways, and the chain has to mean the same
//! thing in both:
//!
//! * ffmpeg sources (RTSP, `PipeWire`) get [`EqChain::ffmpeg_filters`] as
//!   `-af` stages.
//! * The teed microphone (`arecord`) is filtered in process by
//!   [`EqProcessor`].
//!
//! Those are two implementations of one specification, which is exactly the
//! shape of the defect this repository has already paid for twice — the
//! per-source pipeline flags were stored, round-tripped and displayed while
//! reaching nothing, and `AudioPipeline`'s own documentation claimed the
//! daemon honoured them when it did not. So `ffmpeg_filters` emits `width_type
//! q`, matching the `Q` the biquads are designed with rather than ffmpeg's
//! default width in hertz, and `both_backends_describe_the_same_filter` checks
//! the emitted string against the design it is meant to mirror. Where ffmpeg
//! is installed, `the_two_backends_agree_on_real_audio` runs the same signal
//! through both and compares.

use std::fmt;

use crate::audio::biquad::{Biquad, BiquadError};

/// The kinds of section a chain can hold.
///
/// The RBJ cookbook set, minus the all-pass (which changes phase and not
/// magnitude, and has no use here). Each maps onto exactly one ffmpeg filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageKind {
    /// Pass below the corner.
    LowPass,
    /// Pass above the corner. The wind and rumble filter.
    HighPass,
    /// Pass a band around the centre.
    BandPass,
    /// Null the centre, pass everything else. The hum filter.
    Notch,
    /// Boost or cut a bell around the centre.
    Peaking,
    /// Boost or cut everything below the corner.
    LowShelf,
    /// Boost or cut everything above the corner.
    HighShelf,
}

impl StageKind {
    /// The name used in the stored specification.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LowPass => "lowpass",
            Self::HighPass => "highpass",
            Self::BandPass => "bandpass",
            Self::Notch => "notch",
            Self::Peaking => "peaking",
            Self::LowShelf => "lowshelf",
            Self::HighShelf => "highshelf",
        }
    }

    /// The ffmpeg audio filter that implements this kind.
    ///
    /// ffmpeg's names differ from the cookbook's in two places — `equalizer`
    /// for a peaking bell, `bass`/`treble` for the shelves — and getting
    /// either wrong produces a filter graph that fails at runtime on a remote
    /// station rather than at configuration time.
    #[must_use]
    pub const fn ffmpeg_name(self) -> &'static str {
        match self {
            Self::LowPass => "lowpass",
            Self::HighPass => "highpass",
            Self::BandPass => "bandpass",
            Self::Notch => "bandreject",
            Self::Peaking => "equalizer",
            Self::LowShelf => "bass",
            Self::HighShelf => "treble",
        }
    }

    /// Whether `gain_db` means anything for this kind.
    #[must_use]
    pub const fn uses_gain(self) -> bool {
        matches!(self, Self::Peaking | Self::LowShelf | Self::HighShelf)
    }

    /// Parse a stored name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "lowpass" | "lp" => Some(Self::LowPass),
            "highpass" | "hp" => Some(Self::HighPass),
            "bandpass" | "bp" => Some(Self::BandPass),
            "notch" | "bandreject" | "br" => Some(Self::Notch),
            "peaking" | "peak" | "eq" => Some(Self::Peaking),
            "lowshelf" | "bass" => Some(Self::LowShelf),
            "highshelf" | "treble" => Some(Self::HighShelf),
            _ => None,
        }
    }
}

impl fmt::Display for StageKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Default `Q` when a stage does not name one.
///
/// `1/√2` — the Butterworth value, maximally flat, and what every audio tool
/// means by "no resonance".
pub const DEFAULT_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Most passes a single stage may ask for.
///
/// Each pass is another biquad over every sample. Eight is already a 48 dB per
/// octave slope, far steeper than any acoustic argument here calls for, and it
/// bounds what a typo in a settings field can cost a Raspberry Pi.
pub const MAX_PASSES: u8 = 8;

/// One section of the chain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqStage {
    /// Which filter.
    pub kind: StageKind,
    /// Corner or centre frequency, in hertz.
    pub freq_hz: f32,
    /// Resonance. See [`DEFAULT_Q`].
    pub q: f32,
    /// Boost or cut, in decibels. Ignored unless [`StageKind::uses_gain`].
    pub gain_db: f32,
    /// How many times to apply this section, for a steeper slope.
    pub passes: u8,
}

impl EqStage {
    /// A stage with the default `Q`, no gain, and one pass.
    #[must_use]
    pub const fn new(kind: StageKind, freq_hz: f32) -> Self {
        Self {
            kind,
            freq_hz,
            q: DEFAULT_Q,
            gain_db: 0.0,
            passes: 1,
        }
    }

    /// This stage with `passes` applications.
    #[must_use]
    pub const fn with_passes(mut self, passes: u8) -> Self {
        self.passes = passes;
        self
    }

    /// This stage with a `Q`.
    #[must_use]
    pub const fn with_q(mut self, q: f32) -> Self {
        self.q = q;
        self
    }

    /// This stage with a gain, in decibels.
    #[must_use]
    pub const fn with_gain_db(mut self, gain_db: f32) -> Self {
        self.gain_db = gain_db;
        self
    }

    /// Design the biquad for this stage at `sample_rate`.
    ///
    /// # Errors
    ///
    /// [`BiquadError`] when the parameters cannot produce a stable section —
    /// a frequency at or above Nyquist, a non-positive `Q`.
    pub fn design(&self, sample_rate: u32) -> Result<Biquad, BiquadError> {
        match self.kind {
            StageKind::LowPass => Biquad::low_pass(self.freq_hz, sample_rate, self.q),
            StageKind::HighPass => Biquad::high_pass(self.freq_hz, sample_rate, self.q),
            StageKind::BandPass => Biquad::bandpass_q(self.freq_hz, sample_rate, self.q),
            StageKind::Notch => Biquad::notch(self.freq_hz, sample_rate, self.q),
            StageKind::Peaking => Biquad::peaking(self.freq_hz, sample_rate, self.q, self.gain_db),
            StageKind::LowShelf => {
                Biquad::low_shelf(self.freq_hz, sample_rate, self.q, self.gain_db)
            }
            StageKind::HighShelf => {
                Biquad::high_shelf(self.freq_hz, sample_rate, self.q, self.gain_db)
            }
        }
    }

    /// The ffmpeg `-af` fragment for this stage, repeated for its passes.
    ///
    /// `width_type=q` throughout: ffmpeg's default width unit differs per
    /// filter (hertz for `highpass`, and its own `w` for others), so leaving it
    /// out would mean the two backends implement different filters from the
    /// same configuration — silently, and audibly only to a spectrum analyser.
    #[must_use]
    pub fn ffmpeg_fragment(&self) -> String {
        let name = self.kind.ffmpeg_name();
        let f = self.freq_hz;
        let q = self.q;
        let one = if self.kind.uses_gain() {
            format!("{name}=f={f}:width_type=q:width={q}:g={}", self.gain_db)
        } else {
            format!("{name}=f={f}:width_type=q:width={q}")
        };
        std::iter::repeat_n(one, usize::from(self.passes.max(1)))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// The stored form: `kind:freq:q:gain:passes`, trailing defaults omitted.
    #[must_use]
    pub fn to_spec(&self) -> String {
        let mut out = format!("{}:{}", self.kind, trim_float(self.freq_hz));
        let needs_passes = self.passes > 1;
        let needs_gain = self.kind.uses_gain() && (self.gain_db != 0.0 || needs_passes);
        let needs_q = needs_gain || needs_passes || (self.q - DEFAULT_Q).abs() > 1e-6;
        if needs_q {
            out.push(':');
            out.push_str(&trim_float(self.q));
        }
        if needs_gain {
            out.push(':');
            out.push_str(&trim_float(self.gain_db));
        }
        if needs_passes {
            // A gainless kind still needs the placeholder so `passes` lands in
            // the fifth field rather than being read as a gain.
            if !needs_gain {
                out.push_str(":0");
            }
            out.push(':');
            out.push_str(&self.passes.to_string());
        }
        out
    }
}

/// Format a float without a trailing `.0`, so `120` stays `120`.
fn trim_float(v: f32) -> String {
    let s = format!("{v}");
    s.strip_suffix(".0")
        .map_or_else(|| s.clone(), ToOwned::to_owned)
}

/// Why a specification was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EqParseError {
    /// The stage text that could not be read.
    pub stage: String,
    /// What was wrong with it.
    pub reason: &'static str,
}

impl fmt::Display for EqParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot read equaliser stage {:?}: {}",
            self.stage, self.reason
        )
    }
}

impl std::error::Error for EqParseError {}

/// An ordered chain of sections.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EqChain {
    stages: Vec<EqStage>,
}

impl EqChain {
    /// A chain from stages, in order.
    #[must_use]
    pub const fn new(stages: Vec<EqStage>) -> Self {
        Self { stages }
    }

    /// The chain that reproduces the three legacy pipeline booleans.
    ///
    /// This is what every existing station gets at migration, and it must
    /// sound identical to what it heard before. `agc` is deliberately absent:
    /// it is a dynamic-range process, not a filter, and it stays a flag.
    ///
    /// The corners are [`crate::audio::capture::HIGH_PASS_CUTOFF_HZ`] and
    /// [`crate::audio::capture::DC_BLOCK_CUTOFF_HZ`], read from there rather
    /// than repeated, so the two cannot drift.
    #[must_use]
    pub fn from_pipeline_flags(high_pass: bool, dc_removal: bool) -> Self {
        let mut stages = Vec::new();
        if dc_removal {
            stages.push(EqStage::new(
                StageKind::HighPass,
                crate::audio::capture::DC_BLOCK_CUTOFF_HZ,
            ));
        }
        if high_pass {
            stages.push(EqStage::new(
                StageKind::HighPass,
                crate::audio::capture::HIGH_PASS_CUTOFF_HZ,
            ));
        }
        Self { stages }
    }

    /// The stages, in order.
    #[must_use]
    pub fn stages(&self) -> &[EqStage] {
        &self.stages
    }

    /// Whether the chain does nothing.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }

    /// Read a stored specification.
    ///
    /// Stages are separated by `;` or a newline; fields within a stage by `:`,
    /// as `kind:freq[:q[:gain[:passes]]]`. Blank entries and `#` comments are
    /// ignored, so an operator can annotate the field.
    ///
    /// ```text
    /// highpass:120          # wind
    /// notch:50:20           # mains hum
    /// notch:150:20          # its third harmonic
    /// peaking:3500:1:4      # lift where most song lives
    /// ```
    ///
    /// # Errors
    ///
    /// [`EqParseError`] naming the offending stage. A malformed stage is never
    /// skipped: a chain that quietly does less than the operator wrote is the
    /// failure this whole feature exists to stop being invisible.
    pub fn parse(spec: &str) -> Result<Self, EqParseError> {
        let mut stages = Vec::new();
        for raw in spec.split([';', '\n', '\r']) {
            let text = raw.split('#').next().unwrap_or("").trim();
            if text.is_empty() {
                continue;
            }
            stages.push(parse_stage(text)?);
        }
        Ok(Self { stages })
    }

    /// The stored form of the whole chain.
    #[must_use]
    pub fn to_spec(&self) -> String {
        self.stages
            .iter()
            .map(EqStage::to_spec)
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// The ffmpeg `-af` fragments, in order.
    #[must_use]
    pub fn ffmpeg_filters(&self) -> Vec<String> {
        self.stages.iter().map(EqStage::ffmpeg_fragment).collect()
    }

    /// Build the in-process processor for `sample_rate`.
    ///
    /// # Errors
    ///
    /// [`BiquadError`] from the first stage that cannot be designed — most
    /// often a frequency above Nyquist for this source's rate, which is a real
    /// configuration error and not something to filter out silently.
    pub fn build(&self, sample_rate: u32) -> Result<EqProcessor, BiquadError> {
        let mut sections = Vec::new();
        for stage in &self.stages {
            let section = stage.design(sample_rate)?;
            for _ in 0..stage.passes.max(1) {
                sections.push(section);
            }
        }
        Ok(EqProcessor { sections })
    }

    /// The chain's combined magnitude response at `hz`, in decibels.
    ///
    /// For the response curve an operator sees while editing. Cascaded
    /// sections multiply, so their decibels add.
    ///
    /// Returns `0.0` for a stage that cannot be designed at this rate, so a
    /// half-valid chain still draws rather than vanishing.
    #[must_use]
    pub fn magnitude_db_at(&self, hz: f32, sample_rate: u32) -> f64 {
        self.stages
            .iter()
            .map(|stage| {
                stage.design(sample_rate).map_or(0.0, |b| {
                    20.0 * b.magnitude_at(hz, sample_rate).log10() * f64::from(stage.passes.max(1))
                })
            })
            .sum()
    }
}

/// Read one `kind:freq[:q[:gain[:passes]]]`.
fn parse_stage(text: &str) -> Result<EqStage, EqParseError> {
    let err = |reason: &'static str| EqParseError {
        stage: text.to_owned(),
        reason,
    };
    let mut fields = text.split(':').map(str::trim);
    let kind = StageKind::parse(fields.next().unwrap_or_default())
        .ok_or_else(|| err("unknown filter kind"))?;
    let freq_hz: f32 = fields
        .next()
        .ok_or_else(|| err("missing frequency"))?
        .parse()
        .map_err(|_| err("frequency is not a number"))?;
    if !freq_hz.is_finite() || freq_hz <= 0.0 {
        return Err(err("frequency must be above zero"));
    }

    let mut stage = EqStage::new(kind, freq_hz);
    if let Some(q) = fields.next().filter(|s| !s.is_empty()) {
        let q: f32 = q.parse().map_err(|_| err("Q is not a number"))?;
        if !q.is_finite() || q <= 0.0 {
            return Err(err("Q must be above zero"));
        }
        stage.q = q;
    }
    if let Some(gain) = fields.next().filter(|s| !s.is_empty()) {
        let gain: f32 = gain.parse().map_err(|_| err("gain is not a number"))?;
        if !gain.is_finite() {
            return Err(err("gain is not a number"));
        }
        stage.gain_db = gain;
    }
    if let Some(passes) = fields.next().filter(|s| !s.is_empty()) {
        let passes: u8 = passes.parse().map_err(|_| err("passes is not a number"))?;
        if passes == 0 || passes > MAX_PASSES {
            return Err(err("passes must be between 1 and 8"));
        }
        stage.passes = passes;
    }
    if fields.next().is_some() {
        return Err(err("too many fields"));
    }
    Ok(stage)
}

/// A built chain, ready to filter samples.
#[derive(Debug, Clone)]
pub struct EqProcessor {
    sections: Vec<Biquad>,
}

impl EqProcessor {
    /// Filter one sample through every section, in order.
    #[inline]
    #[must_use]
    pub fn process(&mut self, x: f32) -> f32 {
        let mut y = x;
        for section in &mut self.sections {
            #[allow(clippy::cast_possible_truncation)]
            {
                y = section.process(y) as f32;
            }
        }
        y
    }

    /// Filter a buffer in place.
    pub fn process_buffer(&mut self, samples: &mut [f32]) {
        for s in samples.iter_mut() {
            *s = self.process(*s);
        }
    }

    /// Clear every section's state, for a capture discontinuity.
    pub fn reset(&mut self) {
        for section in &mut self.sections {
            section.reset();
        }
    }

    /// How many biquads the chain compiled to, counting passes.
    #[must_use]
    pub const fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// Whether the chain does nothing.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }
}

#[cfg(test)]
mod tests;
