//! How much room the stream directory needs for the sources it holds (MSC-15).
//!
//! Each source writes raw segments into the stream directory, which the disk
//! manager drains by age (`STREAM_RETENTION_SECS`, default 600 s) and by size
//! (`STREAM_MAX_MB`, default 512 MB). Both limits were fixed numbers, set for
//! one source. A 15 s segment of 48 kHz, 16-bit mono audio is 1.44 MB, so ten
//! minutes of one such source is 57.6 MB and five of them are 288 MB: more
//! than the installer's 256 MB tmpfs before either the age drain or the cap
//! acts, and the 512 MB cap is above the mount, so it never fires there.
//!
//! This module is the arithmetic, kept free of I/O so it can be tested: the
//! bytes a set of sources writes over the retention window, the cap that
//! fits a filesystem of a given size, and the backlog that is the same ten
//! minutes per source. The figures are for uncompressed WAV, the format the
//! capture writes by default; a compressed format writes less, so they bound
//! it from above.

/// One source as the stream directory sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRate {
    /// Samples per second.
    pub sample_rate: u32,
    /// Channels written (1 for mono, left or right; 2 for stereo).
    pub channels: u8,
    /// Bits per sample.
    pub bit_depth: u8,
}

impl SourceRate {
    /// Bytes per second of uncompressed PCM.
    #[must_use]
    pub fn bytes_per_sec(self) -> u64 {
        u64::from(self.sample_rate)
            * u64::from(self.channels)
            * u64::from(self.bit_depth).div_ceil(8)
    }

    /// A source as capture writes it. Every source is recorded at 16 bits
    /// whatever its row's `bit_depth` says, and a network stream is resampled
    /// to 48 kHz mono whatever its row's rate and channels say
    /// (`birdnet_core::audio::capture`'s `RECORDED_BIT_DEPTH` and
    /// `NETWORK_SAMPLE_RATE`).
    #[must_use]
    pub fn of(source: &birdnet_db::audio_sources::AudioSource) -> Self {
        use birdnet_core::audio::capture::{NETWORK_SAMPLE_RATE, RECORDED_BIT_DEPTH};
        use birdnet_db::audio_sources::{Channels, SourceKind};
        if source.kind == SourceKind::Rtsp {
            return Self {
                sample_rate: NETWORK_SAMPLE_RATE,
                channels: 1,
                bit_depth: RECORDED_BIT_DEPTH,
            };
        }
        Self {
            sample_rate: source.sample_rate,
            channels: if source.channels == Channels::Stereo {
                2
            } else {
                1
            },
            bit_depth: RECORDED_BIT_DEPTH,
        }
    }
}

/// The share of the stream directory's filesystem the cap may take: the rest
/// is headroom for the segment being written and anything else on the mount.
const CAP_SHARE_OF_FILESYSTEM: f64 = 0.8;

/// Segments per source the shed policy lets wait before it sheds: ten minutes
/// of 15 s segments from one source, the daemon's constant.
#[allow(
    dead_code,
    reason = "read by shed_threshold, which the daemon's shed policy is to call in place of its fixed 40"
)]
pub const BACKLOG_PER_SOURCE: usize = 40;

/// Bytes `sources` write in `retention_secs`.
#[must_use]
pub fn bytes_over(sources: &[SourceRate], retention_secs: u64) -> u64 {
    sources
        .iter()
        .map(|s| s.bytes_per_sec().saturating_mul(retention_secs))
        .fold(0, u64::saturating_add)
}

/// The size cap that fits: what the sources write over the retention window,
/// but never more than [`CAP_SHARE_OF_FILESYSTEM`] of the filesystem.
/// `filesystem_bytes` `None` (it could not be read) leaves the sources' need.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "a filesystem size is far below 2^52 bytes, and the share is positive"
)]
pub fn derived_cap_bytes(
    filesystem_bytes: Option<u64>,
    sources: &[SourceRate],
    retention_secs: u64,
) -> u64 {
    let need = bytes_over(sources, retention_secs);
    filesystem_bytes.map_or(need, |fs| {
        need.min((fs as f64 * CAP_SHARE_OF_FILESYSTEM) as u64)
    })
}

/// The backlog, in segments, at which the shed policy starts: the same ten
/// minutes per source, rather than ten minutes shared by all of them.
#[must_use]
#[allow(
    dead_code,
    reason = "the daemon's shed policy is to call this in place of its fixed 40; until then only tests do"
)]
pub fn shed_threshold(source_count: usize) -> usize {
    BACKLOG_PER_SOURCE.saturating_mul(source_count.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1_000_000;

    fn mono48() -> SourceRate {
        SourceRate {
            sample_rate: 48_000,
            channels: 1,
            bit_depth: 16,
        }
    }

    /// The acceptance gate MSC-15 names: a 256 MB filesystem and five 48 kHz
    /// sources get a cap of at most 205 MB. The fixed default is 512 MB.
    #[test]
    fn five_sources_on_a_256_mb_tmpfs_get_a_cap_that_fits_it() {
        let five = [mono48(); 5];
        assert_eq!(bytes_over(&five, 600), 288 * MB, "the row's arithmetic");
        let cap = derived_cap_bytes(Some(256 * MB), &five, 600);
        assert!(cap <= 205 * MB, "cap {cap} does not fit a 256 MB tmpfs");
        assert!(
            cap > 200 * MB,
            "cap {cap} gives away more than the headroom"
        );
    }

    /// Counterpart: one source on a large disk keeps exactly its ten minutes.
    #[test]
    fn one_source_on_a_large_disk_gets_its_own_need() {
        assert_eq!(
            derived_cap_bytes(Some(32_000 * MB), &[mono48()], 600),
            57_600_000
        );
        assert_eq!(derived_cap_bytes(None, &[mono48()], 600), 57_600_000);
    }

    #[test]
    fn stereo_and_24_bit_take_their_share() {
        let s = SourceRate {
            sample_rate: 48_000,
            channels: 2,
            bit_depth: 24,
        };
        assert_eq!(s.bytes_per_sec(), 288_000);
    }

    #[test]
    fn the_shed_threshold_is_ten_minutes_per_source() {
        assert_eq!(shed_threshold(0), 40);
        assert_eq!(shed_threshold(1), 40);
        assert_eq!(shed_threshold(5), 200);
    }
}
