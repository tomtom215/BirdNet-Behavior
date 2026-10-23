//! How a spectrogram is drawn: the colour map and the dynamic range.
//!
//! Two station-wide settings, read from the settings table on every render so
//! a change reaches the next image rather than the next restart:
//!
//! | Key | Values | Default |
//! |-----|--------|---------|
//! | `spectrogram_colormap` | `viridis`, `magma`, `inferno`, `grey`, `grey_inverted` | `viridis` |
//! | `spectrogram_range_db` | `60`, `80`, `100`, `120` | `80` |
//!
//! The defaults are what every earlier version drew, so a station that never
//! opens the page sees no change.
//!
//! # What the range is
//!
//! The mel power spectrogram is converted to decibels and everything more than
//! `range_db` below the loudest cell is floored to that level
//! (`librosa.power_to_db(top_db=…)`); the image then stretches the surviving
//! span over the whole colour map. A smaller range therefore discards the
//! quietest cells and spends the colour map on the louder ones: more contrast,
//! fewer faint calls. A larger range keeps faint calls and shows the loud ones
//! less sharply. BirdNET-go's `dynamicrange` (`80`/`100`/`120`, sox `-z`)
//! is the same idea; ours adds 60 and keeps 80 as the default.
//!
//! There is no "gain" setting. The image is normalised to its own minimum and
//! maximum, so a gain (a constant added in dB) changes nothing the reader can
//! see; offering one would be a control that does nothing.

use rusqlite::Connection;

use super::colormap::Colormap;

/// Settings key for the colour map.
pub const COLORMAP_KEY: &str = "spectrogram_colormap";
/// Settings key for the dynamic range, in dB.
pub const RANGE_KEY: &str = "spectrogram_range_db";

/// The ranges the settings page offers, in dB.
pub const RANGES_DB: [u16; 4] = [60, 80, 100, 120];
/// The range every earlier version used.
pub const DEFAULT_RANGE_DB: u16 = 80;

/// The look a spectrogram is drawn with. Part of the render cache's key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Look {
    /// The colour map.
    pub colormap: Colormap,
    /// The dynamic range in dB (one of [`RANGES_DB`]).
    pub range_db: u16,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            colormap: Colormap::default(),
            range_db: DEFAULT_RANGE_DB,
        }
    }
}

impl Look {
    /// The look two stored values name; anything that is not a choice (a row
    /// edited by hand, a value from a build with other choices) is the default
    /// for that half, so a bad row degrades the picture and never fails it.
    #[must_use]
    pub fn from_values(colormap: Option<&str>, range_db: Option<&str>) -> Self {
        Self {
            colormap: colormap.and_then(Colormap::parse).unwrap_or_default(),
            range_db: range_db.and_then(parse_range).unwrap_or(DEFAULT_RANGE_DB),
        }
    }

    /// Read the look from the settings table.
    pub fn load(conn: &Connection) -> Self {
        let get = |key| birdnet_db::settings::get(conn, key).ok();
        Self::from_values(get(COLORMAP_KEY).as_deref(), get(RANGE_KEY).as_deref())
    }

    /// A short token naming the look, for the response's `ETag`.
    #[must_use]
    pub fn tag(self) -> String {
        format!("{}-{}", self.colormap.as_str(), self.range_db)
    }
}

fn parse_range(value: &str) -> Option<u16> {
    let n: u16 = value.trim().parse().ok()?;
    RANGES_DB.contains(&n).then_some(n)
}

/// Every colour map as `(stored value, label)`, in the order the page lists them.
#[must_use]
pub fn colormap_choices() -> Vec<(&'static str, &'static str)> {
    Colormap::ALL
        .iter()
        .map(|m| (m.as_str(), m.label()))
        .collect()
}

/// Whether `value` names a colour map (for the settings validator).
#[must_use]
pub fn is_colormap(value: &str) -> bool {
    Colormap::parse(value).is_some()
}

/// Whether `value` is one of the offered ranges (for the settings validator).
#[must_use]
pub fn is_range_db(value: &str) -> bool {
    parse_range(value).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_what_every_earlier_version_drew() {
        let look = Look::default();
        assert_eq!(look.colormap, Colormap::Viridis);
        assert_eq!(look.range_db, 80);
        assert_eq!(Look::from_values(None, None), look);
    }

    #[test]
    fn a_value_that_is_no_choice_is_the_default_for_its_half_only() {
        let look = Look::from_values(Some("rainbow"), Some("100"));
        assert_eq!(look.colormap, Colormap::Viridis);
        assert_eq!(look.range_db, 100);
        let look = Look::from_values(Some("magma"), Some("7"));
        assert_eq!(look.colormap, Colormap::Magma);
        assert_eq!(look.range_db, 80);
        assert_eq!(Look::from_values(Some("magma"), Some("abc")).range_db, 80);
    }

    #[test]
    fn the_validators_accept_exactly_the_choices() {
        for m in Colormap::ALL {
            assert!(is_colormap(m.as_str()));
        }
        assert!(!is_colormap("") && !is_colormap("rainbow"));
        for r in RANGES_DB {
            assert!(is_range_db(&r.to_string()));
        }
        for bad in ["", "0", "59", "81", "121", "-80", "80.5", "eighty"] {
            assert!(!is_range_db(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_default_range_is_one_the_page_offers() {
        assert!(RANGES_DB.contains(&DEFAULT_RANGE_DB));
    }

    #[test]
    fn tags_differ_when_either_half_does() {
        let a = Look::default();
        let b = Look {
            colormap: Colormap::Magma,
            ..a
        };
        let c = Look { range_db: 100, ..a };
        assert_ne!(a.tag(), b.tag());
        assert_ne!(a.tag(), c.tag());
        assert_ne!(b.tag(), c.tag());
    }
}
