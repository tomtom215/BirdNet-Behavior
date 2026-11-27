//! The station's profile: what its defaults are for.
//!
//! `STATION_PROFILE=hobby` (the default, and every station that never set the
//! key) keeps the defaults a garden station has always had. `research` changes
//! the defaults where a study needs a stronger guarantee than a hobbyist does,
//! and only the defaults: a setting the operator wrote down is theirs, under
//! either profile. Choosing `research` says nothing about the station's
//! accuracy; it asks for the evidence a study needs to be kept.

use super::Config;

/// What a station's defaults are for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StationProfile {
    /// A garden station: today's defaults.
    Hobby,
    /// A station whose records are evidence for a study.
    Research,
}

impl StationProfile {
    /// The profile `config` names; `Hobby` when it names none. A value that
    /// is neither word is logged and read as `Hobby`, which is what its
    /// absence means.
    #[must_use]
    pub fn of(config: Option<&Config>) -> Self {
        let Some(raw) = config.and_then(|c| c.get("STATION_PROFILE")) else {
            return Self::Hobby;
        };
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "hobby" => Self::Hobby,
            "research" => Self::Research,
            other => {
                tracing::warn!(
                    value = other,
                    "STATION_PROFILE is neither hobby nor research; using the hobby defaults"
                );
                Self::Hobby
            }
        }
    }

    /// The profile's name, as the key takes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hobby => "hobby",
            Self::Research => "research",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(value: &str) -> Config {
        Config::parse(&format!("STATION_PROFILE={value}\n")).unwrap()
    }

    #[test]
    fn no_profile_is_the_hobby_profile_and_research_is_research() {
        assert_eq!(StationProfile::of(None), StationProfile::Hobby);
        assert_eq!(
            StationProfile::of(Some(&with("research"))),
            StationProfile::Research
        );
        assert_eq!(
            StationProfile::of(Some(&with(" Research "))),
            StationProfile::Research
        );
        assert_eq!(
            StationProfile::of(Some(&with("hobby"))),
            StationProfile::Hobby
        );
        assert_eq!(
            StationProfile::of(Some(&with("lab"))),
            StationProfile::Hobby
        );
    }
}
