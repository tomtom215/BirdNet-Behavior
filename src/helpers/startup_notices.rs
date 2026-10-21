//! One-time start-up notices about keys that now do what they say.
//!
//! Three keys in an existing `birdnet.conf` changed meaning between releases
//! without the file changing: `APPRISE_TRIGGER=off`, `RECORDING_LENGTH` and
//! `FULL_DISK=keep`. Each used to be read as nothing, or as "each", and now
//! takes effect. A station that upgraded with one of them set is doing something
//! different from last week, and nothing on screen says so; the journal should.
//!
//! The functions return the sentences and the caller logs them, so the text is
//! testable without capturing a log.

use birdnet_core::config::Config;

/// The chunk length the station uses when none is set, in seconds.
const DEFAULT_SEGMENT_SECS: u64 = 15;

/// The notices for a configuration file, in the order they are logged. Empty
/// for a file that sets none of the keys, or sets them to what they always did.
#[must_use]
pub fn notices(config: Option<&Config>) -> Vec<String> {
    let Some(config) = config else {
        return Vec::new();
    };
    let value = |key: &str| config.get(key).map(str::trim).filter(|v| !v.is_empty());
    let mut out = Vec::new();

    // `Config::parse` copies RECORDING_LENGTH into SEGMENT_DURATION when the
    // latter is blank; mirror that, and say nothing when the value is the
    // default the key would have produced anyway.
    if let Some(length) = value("RECORDING_LENGTH")
        && let Ok(secs) = length.parse::<u64>()
        && secs != DEFAULT_SEGMENT_SECS
        && config.get("SEGMENT_DURATION") == Some(length)
    {
        out.push(format!(
            "RECORDING_LENGTH={secs} is now read: recordings are cut into {secs}-second chunks \
             (BirdNET-Pi's name for SEGMENT_DURATION). Earlier versions ignored this key and \
             used SEGMENT_DURATION, or {DEFAULT_SEGMENT_SECS} seconds when that was not set. \
             To change the length, set SEGMENT_DURATION"
        ));
    }

    if value("FULL_DISK").is_some_and(|v| v.eq_ignore_ascii_case("keep")) {
        out.push(
            "FULL_DISK=keep is now in force: when the disk reaches the purge threshold the \
             station stops recording and deletes nothing. Earlier versions deleted the oldest \
             clips regardless of this key. To delete the oldest clips instead, set FULL_DISK=purge"
                .to_owned(),
        );
    }
    out
}

/// What `FULL_DISK=keep` cannot do when the stream directory *is* the recordings
/// directory (`--watch-dir` set to it): the one disk manager watching it is the
/// stream manager, which always purges, so the setting is not in force.
#[must_use]
pub fn keep_overridden_by_watch_dir(
    action: birdnet_core::audio::capture::FullDiskAction,
    watch_dir_is_recordings_dir: bool,
) -> Option<String> {
    (action == birdnet_core::audio::capture::FullDiskAction::Keep && watch_dir_is_recordings_dir)
        .then(|| {
            "FULL_DISK=keep is NOT in force: --watch-dir is the recordings directory, and the \
             manager for a watch directory always deletes the oldest files when the disk is \
             full. Point --watch-dir at a different directory (a RAM disk is usual) for 'keep' \
             to hold"
                .to_owned()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers::test_support::config_with;
    use birdnet_core::audio::capture::FullDiskAction;

    #[test]
    fn recording_length_and_full_disk_keep_are_announced() {
        let cfg = config_with(&[("RECORDING_LENGTH", "30"), ("FULL_DISK", "keep")]);
        let n = notices(Some(&cfg));
        assert_eq!(n.len(), 2, "{n:?}");
        assert!(
            n[0].contains("RECORDING_LENGTH=30") && n[0].contains("30-second"),
            "{n:?}"
        );
        assert!(n[1].contains("FULL_DISK=keep") && n[1].contains("deletes nothing"));
    }

    /// The counterparts: a file that sets nothing, or sets what it always did,
    /// says nothing, and an explicit `SEGMENT_DURATION` means `RECORDING_LENGTH` is
    /// not what is in force.
    #[test]
    fn a_file_that_changed_nothing_says_nothing() {
        assert!(notices(None).is_empty());
        assert!(notices(Some(&config_with(&[]))).is_empty());
        let same = config_with(&[("RECORDING_LENGTH", "15"), ("FULL_DISK", "purge")]);
        assert!(notices(Some(&same)).is_empty());
        let explicit = config_with(&[("RECORDING_LENGTH", "30"), ("SEGMENT_DURATION", "20")]);
        assert!(notices(Some(&explicit)).is_empty());
        let blank = config_with(&[("RECORDING_LENGTH", " "), ("FULL_DISK", "")]);
        assert!(notices(Some(&blank)).is_empty());
    }

    #[test]
    fn keep_is_reported_overridden_only_when_it_is() {
        assert!(keep_overridden_by_watch_dir(FullDiskAction::Keep, true).is_some());
        assert!(keep_overridden_by_watch_dir(FullDiskAction::Keep, false).is_none());
        assert!(keep_overridden_by_watch_dir(FullDiskAction::Purge, true).is_none());
    }
}
