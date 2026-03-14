//! Notification template and trigger system.
//!
//! Provides:
//! - [`NotificationTemplate`] for variable substitution in notification messages
//! - [`NotificationContext`] holding all detection metadata for template rendering
//! - [`TriggerMode`] controlling when notifications fire
//! - [`DetectionCounter`] trait for querying detection counts from the database

use std::collections::HashSet;
use std::fmt;

/// Context for rendering notification templates.
///
/// Contains all the variables that BirdNET-Pi supports for notification
/// template substitution.
#[derive(Debug, Clone)]
pub struct NotificationContext {
    /// Scientific name of the detected species.
    pub sci_name: String,
    /// Common name of the detected species.
    pub com_name: String,
    /// Detection confidence (0.0 - 1.0).
    pub confidence: f32,
    /// Detection confidence as percentage (0 - 100).
    pub confidence_pct: u32,
    /// Detection date (YYYY-MM-DD).
    pub date: String,
    /// Detection time (HH:MM:SS).
    pub time: String,
    /// ISO week number.
    pub week: u32,
    /// Station latitude.
    pub latitude: f64,
    /// Station longitude.
    pub longitude: f64,
    /// Detection reason / notes.
    pub reason: String,
    /// URL to listen to the detection audio.
    pub listen_url: Option<String>,
    /// URL to a species image.
    pub image_url: Option<String>,
    /// URL to the station web interface.
    pub station_url: Option<String>,
}

/// A notification message template with variable substitution.
///
/// Templates use BirdNET-Pi-compatible `$variable` syntax:
/// `$sciname`, `$comname`, `$confidence`, `$confidencepct`, `$listenurl`,
/// `$friendlyurl`, `$date`, `$time`, `$week`, `$latitude`, `$longitude`,
/// `$cutoff`, `$sens`, `$overlap`, `$flickrimage`/`$image`, `$reason`.
#[derive(Debug, Clone)]
pub struct NotificationTemplate {
    /// Title template string.
    title_template: String,
    /// Body template string.
    body_template: String,
}

impl Default for NotificationTemplate {
    fn default() -> Self {
        Self {
            title_template: "Bird Detection: $comname".to_string(),
            body_template:
                "$comname ($sciname) detected ($confidencepct% confidence) at $time on $date"
                    .to_string(),
        }
    }
}

impl NotificationTemplate {
    /// Create a new template with the given title and body patterns.
    #[must_use]
    pub fn new(title_template: String, body_template: String) -> Self {
        Self {
            title_template,
            body_template,
        }
    }

    /// Render the template with the given context, returning `(title, body)`.
    #[must_use]
    pub fn render(&self, ctx: &NotificationContext) -> (String, String) {
        let title = substitute(&self.title_template, ctx);
        let body = substitute(&self.body_template, ctx);
        (title, body)
    }

    /// Get the title template string.
    #[must_use]
    pub fn title_template(&self) -> &str {
        &self.title_template
    }

    /// Get the body template string.
    #[must_use]
    pub fn body_template(&self) -> &str {
        &self.body_template
    }
}

/// Perform variable substitution on a template string.
fn substitute(template: &str, ctx: &NotificationContext) -> String {
    template
        .replace("$sciname", &ctx.sci_name)
        .replace("$comname", &ctx.com_name)
        .replace("$confidencepct", &ctx.confidence_pct.to_string())
        .replace("$confidence", &format!("{:.2}", ctx.confidence))
        .replace("$date", &ctx.date)
        .replace("$time", &ctx.time)
        .replace("$week", &ctx.week.to_string())
        .replace("$latitude", &format!("{:.6}", ctx.latitude))
        .replace("$longitude", &format!("{:.6}", ctx.longitude))
        .replace("$reason", &ctx.reason)
        .replace(
            "$listenurl",
            ctx.listen_url.as_deref().unwrap_or(""),
        )
        .replace(
            "$friendlyurl",
            ctx.station_url.as_deref().unwrap_or(""),
        )
        .replace(
            "$flickrimage",
            ctx.image_url.as_deref().unwrap_or(""),
        )
        .replace(
            "$image",
            ctx.image_url.as_deref().unwrap_or(""),
        )
        // Placeholders for settings that may not be available in context
        .replace("$cutoff", "")
        .replace("$sens", "")
        .replace("$overlap", "")
}

/// Notification trigger modes controlling when notifications are sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerMode {
    /// Send on every detection (default, existing behavior).
    EachDetection,
    /// Send when a species has fewer than 5 detections this week.
    NewSpecies,
    /// Send on the first detection of each species per day.
    NewSpeciesDaily,
}

impl TriggerMode {
    /// Parse a trigger mode from a string.
    ///
    /// Accepts: `"each"`, `"new-species"`, `"new-species-daily"`.
    /// Unknown values fall back to `EachDetection`.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "new-species" | "new_species" => Self::NewSpecies,
            "new-species-daily" | "new_species_daily" => Self::NewSpeciesDaily,
            _ => Self::EachDetection,
        }
    }
}

impl fmt::Display for TriggerMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EachDetection => write!(f, "each"),
            Self::NewSpecies => write!(f, "new-species"),
            Self::NewSpeciesDaily => write!(f, "new-species-daily"),
        }
    }
}

/// Trait for querying detection counts from the database.
///
/// Used by the notification system to implement `new-species` and
/// `new-species-daily` trigger modes.
pub trait DetectionCounter: Send + Sync {
    /// Count detections for a species today.
    fn todays_count_for(&self, sci_name: &str) -> u64;
    /// Count detections for a species this week (ISO week).
    fn this_weeks_count_for(&self, sci_name: &str) -> u64;
}

/// Species filter for notifications.
#[derive(Debug, Clone)]
pub struct SpeciesFilter {
    /// Species to exclude (scientific names).
    exclude: HashSet<String>,
    /// Species to include exclusively (scientific names). Empty = all.
    only: HashSet<String>,
}

impl SpeciesFilter {
    /// Create a new species filter from comma-separated lists.
    #[must_use]
    pub fn new(exclude: Option<&str>, only: Option<&str>) -> Self {
        Self {
            exclude: parse_species_list(exclude),
            only: parse_species_list(only),
        }
    }

    /// Returns `true` if the species passes the filter.
    #[must_use]
    pub fn is_allowed(&self, sci_name: &str) -> bool {
        if self.exclude.contains(sci_name) {
            return false;
        }
        if !self.only.is_empty() && !self.only.contains(sci_name) {
            return false;
        }
        true
    }
}

/// Parse a comma-separated species list into a `HashSet`.
fn parse_species_list(input: Option<&str>) -> HashSet<String> {
    input
        .map(|s| {
            s.split(',')
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Full notification filter that combines trigger mode, species filter,
/// and detection counting.
#[derive(Debug)]
pub struct NotificationFilter {
    /// Trigger mode.
    pub trigger: TriggerMode,
    /// Species include/exclude filter.
    pub species_filter: SpeciesFilter,
}

impl NotificationFilter {
    /// Check if a detection should trigger a notification.
    ///
    /// `counter` is used for `NewSpecies` and `NewSpeciesDaily` trigger modes
    /// to query historical detection counts.
    pub fn should_notify(
        &self,
        sci_name: &str,
        counter: Option<&dyn DetectionCounter>,
    ) -> bool {
        // Species filter check.
        if !self.species_filter.is_allowed(sci_name) {
            return false;
        }

        // Trigger mode check.
        match self.trigger {
            TriggerMode::EachDetection => true,
            TriggerMode::NewSpecies => {
                let count = counter
                    .map(|c| c.this_weeks_count_for(sci_name))
                    .unwrap_or(0);
                // Notify when species has fewer than 5 detections this week.
                // The current detection has not been inserted yet, so count
                // represents previous detections.
                count < 5
            }
            TriggerMode::NewSpeciesDaily => {
                let count = counter
                    .map(|c| c.todays_count_for(sci_name))
                    .unwrap_or(0);
                // First detection of the day for this species.
                count == 0
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_context() -> NotificationContext {
        NotificationContext {
            sci_name: "Erithacus rubecula".to_string(),
            com_name: "European Robin".to_string(),
            confidence: 0.92,
            confidence_pct: 92,
            date: "2026-03-14".to_string(),
            time: "08:30:00".to_string(),
            week: 11,
            latitude: 51.5074,
            longitude: -0.1278,
            reason: "high confidence".to_string(),
            listen_url: Some("http://localhost:8502/listen/123".to_string()),
            image_url: Some("http://example.com/robin.jpg".to_string()),
            station_url: Some("http://localhost:8502".to_string()),
        }
    }

    #[test]
    fn default_template_renders() {
        let template = NotificationTemplate::default();
        let ctx = sample_context();
        let (title, body) = template.render(&ctx);
        assert_eq!(title, "Bird Detection: European Robin");
        assert!(body.contains("European Robin"));
        assert!(body.contains("Erithacus rubecula"));
        assert!(body.contains("92%"));
        assert!(body.contains("08:30:00"));
        assert!(body.contains("2026-03-14"));
    }

    #[test]
    fn custom_template_renders() {
        let template = NotificationTemplate::new(
            "$comname spotted!".to_string(),
            "A $comname ($sciname) was detected at $time with $confidencepct% confidence.".to_string(),
        );
        let ctx = sample_context();
        let (title, body) = template.render(&ctx);
        assert_eq!(title, "European Robin spotted!");
        assert!(body.contains("92% confidence"));
    }

    #[test]
    fn template_missing_optional_urls() {
        let template = NotificationTemplate::new(
            "$comname".to_string(),
            "Listen: $listenurl Image: $image".to_string(),
        );
        let mut ctx = sample_context();
        ctx.listen_url = None;
        ctx.image_url = None;
        let (_, body) = template.render(&ctx);
        assert_eq!(body, "Listen:  Image: ");
    }

    #[test]
    fn trigger_mode_parse() {
        assert_eq!(TriggerMode::parse("each"), TriggerMode::EachDetection);
        assert_eq!(TriggerMode::parse("new-species"), TriggerMode::NewSpecies);
        assert_eq!(
            TriggerMode::parse("new-species-daily"),
            TriggerMode::NewSpeciesDaily
        );
        assert_eq!(
            TriggerMode::parse("new_species_daily"),
            TriggerMode::NewSpeciesDaily
        );
        assert_eq!(TriggerMode::parse("unknown"), TriggerMode::EachDetection);
    }

    #[test]
    fn species_filter_exclude() {
        let filter = SpeciesFilter::new(Some("Corvus corax,Pica pica"), None);
        assert!(!filter.is_allowed("Corvus corax"));
        assert!(!filter.is_allowed("Pica pica"));
        assert!(filter.is_allowed("Erithacus rubecula"));
    }

    #[test]
    fn species_filter_only() {
        let filter = SpeciesFilter::new(None, Some("Erithacus rubecula,Parus major"));
        assert!(filter.is_allowed("Erithacus rubecula"));
        assert!(filter.is_allowed("Parus major"));
        assert!(!filter.is_allowed("Corvus corax"));
    }

    #[test]
    fn species_filter_empty_allows_all() {
        let filter = SpeciesFilter::new(None, None);
        assert!(filter.is_allowed("Anything"));
    }

    #[test]
    fn species_filter_exclude_takes_precedence() {
        let filter = SpeciesFilter::new(
            Some("Erithacus rubecula"),
            Some("Erithacus rubecula,Parus major"),
        );
        // Excluded even though it's in the "only" list.
        assert!(!filter.is_allowed("Erithacus rubecula"));
        assert!(filter.is_allowed("Parus major"));
    }

    struct MockCounter {
        daily: u64,
        weekly: u64,
    }

    impl DetectionCounter for MockCounter {
        fn todays_count_for(&self, _sci_name: &str) -> u64 {
            self.daily
        }
        fn this_weeks_count_for(&self, _sci_name: &str) -> u64 {
            self.weekly
        }
    }

    #[test]
    fn trigger_each_always_fires() {
        let filter = NotificationFilter {
            trigger: TriggerMode::EachDetection,
            species_filter: SpeciesFilter::new(None, None),
        };
        let counter = MockCounter { daily: 100, weekly: 500 };
        assert!(filter.should_notify("Any", Some(&counter)));
    }

    #[test]
    fn trigger_new_species_respects_weekly_count() {
        let filter = NotificationFilter {
            trigger: TriggerMode::NewSpecies,
            species_filter: SpeciesFilter::new(None, None),
        };
        let counter_low = MockCounter { daily: 0, weekly: 3 };
        assert!(filter.should_notify("Robin", Some(&counter_low)));

        let counter_high = MockCounter { daily: 0, weekly: 5 };
        assert!(!filter.should_notify("Robin", Some(&counter_high)));
    }

    #[test]
    fn trigger_new_species_daily_respects_daily_count() {
        let filter = NotificationFilter {
            trigger: TriggerMode::NewSpeciesDaily,
            species_filter: SpeciesFilter::new(None, None),
        };
        let counter_zero = MockCounter { daily: 0, weekly: 50 };
        assert!(filter.should_notify("Robin", Some(&counter_zero)));

        let counter_nonzero = MockCounter { daily: 1, weekly: 50 };
        assert!(!filter.should_notify("Robin", Some(&counter_nonzero)));
    }

    #[test]
    fn trigger_without_counter_defaults_to_allow() {
        let filter = NotificationFilter {
            trigger: TriggerMode::NewSpeciesDaily,
            species_filter: SpeciesFilter::new(None, None),
        };
        // No counter available — should default to allowing.
        assert!(filter.should_notify("Robin", None));
    }

    #[test]
    fn species_filter_blocks_before_trigger() {
        let filter = NotificationFilter {
            trigger: TriggerMode::EachDetection,
            species_filter: SpeciesFilter::new(Some("Corvus corax"), None),
        };
        assert!(!filter.should_notify("Corvus corax", None));
        assert!(filter.should_notify("Erithacus rubecula", None));
    }
}
