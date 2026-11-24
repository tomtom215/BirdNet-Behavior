//! Whether a source's sample rate can carry the band a classifier listens to
//! (ACC-14).
//!
//! Resampling up adds samples, not sound. An 8 kHz camera stream resampled to
//! the 48 kHz a classifier wants still holds nothing above 4 kHz, and a bird
//! singing above that is not quiet in it but absent. The station looks healthy
//! — capture runs, recordings are analysed, lower-pitched species are detected
//! — and the high-pitched ones are simply never reported. Nothing else says
//! so, which is what this module is for.

use std::collections::HashSet;
use std::fmt;

/// The highest frequency, in hertz, a source must be able to carry before it
/// is not worth a warning: a Nyquist frequency (half the sample rate) below
/// this is reported.
///
/// **A judgement, not a measured or cited figure.** Many passerine songs have
/// energy between 8 and 12 kHz (high-pitched warblers, kinglets and
/// goldcrests, some treecreepers), so a source whose band stops below 12 kHz
/// can miss them outright, while one that reaches 12 kHz carries most of what
/// the classifiers this project ships are used to detect. A 24 kHz source is
/// the lowest rate that passes; 22.05 kHz (11.025 kHz Nyquist) and the
/// 8/16 kHz rates common on network cameras do not. Moving it changes only
/// which sources get a warning, never what is analysed.
pub const MIN_USEFUL_NYQUIST_HZ: u32 = 12_000;

/// A source that cannot carry the band its classifier listens to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BandWarning {
    /// The source's sample rate, in hertz.
    pub source_rate_hz: u32,
    /// The classifier's sample rate, in hertz.
    pub model_rate_hz: u32,
}

impl BandWarning {
    /// The highest frequency the source carries: half its sample rate.
    #[must_use]
    pub const fn source_nyquist_hz(&self) -> u32 {
        self.source_rate_hz / 2
    }

    /// The highest frequency the classifier listens to: half its sample rate.
    #[must_use]
    pub const fn model_nyquist_hz(&self) -> u32 {
        self.model_rate_hz / 2
    }
}

impl fmt::Display for BandWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "this source is sampled at {} Hz, so it carries nothing above {:.1} kHz, and the \
             classifier listens up to {:.1} kHz; birds singing above {:.1} kHz (many warblers, \
             goldcrests and kinglets) cannot be detected from it. Capture it at 32 or 48 kHz if \
             the device or camera offers it",
            self.source_rate_hz,
            f64::from(self.source_nyquist_hz()) / 1000.0,
            f64::from(self.model_nyquist_hz()) / 1000.0,
            f64::from(self.source_nyquist_hz()) / 1000.0,
        )
    }
}

/// A warning when a source sampled at `source_rate_hz` cannot carry the band
/// a classifier sampled at `model_rate_hz` listens to, or `None` when it can.
///
/// Warns when the source's Nyquist frequency is below both
/// [`MIN_USEFUL_NYQUIST_HZ`] and the classifier's: a source that matches a
/// low-rate classifier loses nothing that classifier would have heard. A rate
/// of zero is not a rate and gets no verdict.
#[must_use]
pub const fn band_warning(source_rate_hz: u32, model_rate_hz: u32) -> Option<BandWarning> {
    if source_rate_hz == 0 || model_rate_hz == 0 {
        return None;
    }
    let source_nyquist = source_rate_hz / 2;
    if source_nyquist < MIN_USEFUL_NYQUIST_HZ && source_nyquist < model_rate_hz / 2 {
        Some(BandWarning {
            source_rate_hz,
            model_rate_hz,
        })
    } else {
        None
    }
}

/// One verdict per source and rate.
///
/// The first recording of a source at a rate is judged, and later ones at the
/// same rate are not, so a narrow source is reported once rather than on every
/// segment. A source that comes back at another rate (its settings changed) is
/// judged again.
#[derive(Debug, Default)]
pub struct BandWatch {
    judged: HashSet<(String, u32)>,
}

impl BandWatch {
    /// The warning for `source` at `source_rate_hz` against `model_rate_hz`,
    /// the first time this source is seen at this rate; `None` after that,
    /// and `None` whenever the source can carry the band.
    pub fn first_look(
        &mut self,
        source: &str,
        source_rate_hz: u32,
        model_rate_hz: u32,
    ) -> Option<BandWarning> {
        if !self.judged.insert((source.to_owned(), source_rate_hz)) {
            return None;
        }
        band_warning(source_rate_hz, model_rate_hz)
    }
}

#[cfg(test)]
mod tests {
    use super::{BandWatch, MIN_USEFUL_NYQUIST_HZ, band_warning};

    /// The ACC-14 gate: an 8 or 16 kHz camera stream warns against a 48 kHz
    /// classifier.
    #[test]
    fn a_narrow_source_warns_against_a_48_khz_model() {
        for source in [8_000, 16_000, 22_050] {
            let w = band_warning(source, 48_000).unwrap_or_else(|| panic!("{source} Hz"));
            assert_eq!(w.source_nyquist_hz(), source / 2);
            assert_eq!(w.model_nyquist_hz(), 24_000);
            assert!(w.to_string().contains(&format!("{source} Hz")), "{w}");
        }
    }

    /// The counterpart: sources that carry the band say nothing.
    #[test]
    fn a_source_that_carries_the_band_does_not_warn() {
        assert_eq!(band_warning(48_000, 48_000), None);
        assert_eq!(band_warning(32_000, 48_000), None);
        assert_eq!(band_warning(44_100, 48_000), None);
        assert_eq!(band_warning(96_000, 48_000), None);
    }

    /// A source no narrower than its classifier loses nothing that classifier
    /// would hear: a 24 kHz source against a 32 kHz model reaches 12 kHz.
    #[test]
    fn the_model_s_band_bounds_what_a_source_must_carry() {
        assert_eq!(band_warning(24_000, 32_000), None);
        assert_eq!(band_warning(16_000, 16_000), None);
        assert_eq!(
            band_warning(8_000, 16_000).map(|w| w.source_rate_hz),
            Some(8_000)
        );
    }

    /// The bound itself, on both sides.
    #[test]
    fn the_bound_is_twelve_kilohertz_of_nyquist() {
        let at = 2 * MIN_USEFUL_NYQUIST_HZ;
        assert_eq!(band_warning(at, 48_000), None);
        assert!(band_warning(at - 2, 48_000).is_some());
        assert_eq!(band_warning(0, 48_000), None);
        assert_eq!(band_warning(8_000, 0), None);
    }

    /// One verdict per source and rate.
    #[test]
    fn a_source_is_judged_once_per_rate() {
        let mut watch = BandWatch::default();
        assert!(watch.first_look("cam", 16_000, 48_000).is_some());
        assert!(watch.first_look("cam", 16_000, 48_000).is_none());
        assert!(watch.first_look("other", 16_000, 48_000).is_some());
        assert!(watch.first_look("cam", 48_000, 48_000).is_none());
        assert!(watch.first_look("cam", 8_000, 48_000).is_some());
    }
}
