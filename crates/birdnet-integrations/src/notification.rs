//! Notification template and trigger system.
//!
//! Provides:
//! - [`NotificationTemplate`] for variable substitution in notification messages
//! - [`NotificationContext`] holding all detection metadata for template rendering
//! - [`TriggerMode`] controlling when notifications fire
//! - [`DetectionCounter`] trait for querying detection counts from the database

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;

/// Context for rendering notification templates.
///
/// Contains all the variables that `BirdNET-Pi` supports for notification
/// template substitution.
///
/// A value the station does not have is `None` and renders as nothing. It is
/// never a made-up default: a station with no coordinates must not tell a
/// phone it is at 0.000000, 0.000000.
#[derive(Debug, Clone)]
pub struct NotificationContext {
    /// Scientific name of the detected species.
    pub sci_name: String,
    /// Common name of the detected species, as the classifier names it and as
    /// it is stored. It is the key: the detection's page is found by it, so the
    /// links are built from it.
    pub com_name: String,
    /// The name to read, when the station shows common names in another
    /// language (`$comname`); `None` shows [`Self::com_name`]. Never used in a
    /// link — see [`Self::com_name`].
    pub shown_name: Option<String>,
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
    /// Station latitude, when the station has one.
    pub latitude: Option<f64>,
    /// Station longitude, when the station has one.
    pub longitude: Option<f64>,
    /// Why this detection produced a notification (`$reason`); see
    /// [`NotificationFilter::notify_reason`].
    pub reason: String,
    /// The confidence bar this detection cleared (`$cutoff`).
    pub cutoff: Option<f64>,
    /// BirdNET sensitivity the run used (`$sens`).
    pub sensitivity: Option<f64>,
    /// Analysis-window overlap in seconds the run used (`$overlap`).
    pub overlap: Option<f64>,
    /// The detection's own page (`$listenurl`; `$friendlyurl` links to it).
    /// Filled from the station's address by [`NotificationTemplate`] when it
    /// has one and this is `None`.
    pub listen_url: Option<String>,
    /// URL of a picture of the species (`$image`). Filled the same way.
    pub image_url: Option<String>,
    /// URL of the station web interface.
    pub station_url: Option<String>,
}

impl NotificationContext {
    /// Point the links this context does not already carry at `station_url`:
    /// the detection's own page, and the species picture the station serves.
    ///
    /// `listen_url`, `image_url` and `station_url` that are already set are
    /// left alone, so a caller that knows better wins.
    fn fill_links(&mut self, station_url: &str) {
        use crate::offsite::sigv4::encode_component as enc;
        let base = station_url.trim_end_matches('/');
        if base.is_empty() {
            return;
        }
        if self.station_url.is_none() {
            self.station_url = Some(base.to_owned());
        }
        if self.listen_url.is_none() {
            self.listen_url = Some(format!(
                "{base}/detections/detail?date={}&time={}&name={}",
                enc(&self.date),
                enc(&self.time),
                enc(&self.com_name),
            ));
        }
        if self.image_url.is_none() {
            self.image_url = Some(format!(
                "{base}/api/v2/species/image/{}/file",
                enc(&self.sci_name)
            ));
        }
    }
}

/// A notification message template with variable substitution.
///
/// Templates use `BirdNET-Pi`-compatible `$variable` syntax:
/// `$sciname`, `$comname`, `$confidence`, `$confidencepct`, `$listenurl`,
/// `$friendlyurl`, `$date`, `$time`, `$week`, `$latitude`, `$longitude`,
/// `$cutoff`, `$sens`, `$overlap`, `$flickrimage`/`$image`, `$reason`.
#[derive(Debug, Clone)]
pub struct NotificationTemplate {
    /// Title template string.
    title_template: String,
    /// Body template string.
    body_template: String,
    /// How the station is reached from the device that reads the
    /// notification, when known; the source of `$listenurl`, `$friendlyurl`
    /// and `$image`.
    station_url: Option<String>,
}

impl Default for NotificationTemplate {
    fn default() -> Self {
        Self {
            title_template: "Bird Detection: $comname".to_string(),
            body_template:
                "$comname ($sciname) detected ($confidencepct% confidence) at $time on $date"
                    .to_string(),
            station_url: None,
        }
    }
}

impl NotificationTemplate {
    /// Create a new template with the given title and body patterns.
    #[must_use]
    pub const fn new(title_template: String, body_template: String) -> Self {
        Self {
            title_template,
            body_template,
            station_url: None,
        }
    }

    /// Say how the station is reached from outside, so `$listenurl`,
    /// `$friendlyurl` and `$image` have something to point at. Without it
    /// they render as nothing — a link to a guessed address is worse than none.
    #[must_use]
    pub fn with_station_url(mut self, station_url: Option<String>) -> Self {
        self.station_url = station_url.filter(|u| !u.trim().is_empty());
        self
    }

    /// Render the template with the given context, returning `(title, body)`.
    #[must_use]
    pub fn render(&self, ctx: &NotificationContext) -> (String, String) {
        let ctx = self
            .station_url
            .as_deref()
            .map_or(Cow::Borrowed(ctx), |base| {
                let mut linked = ctx.clone();
                linked.fill_links(base);
                Cow::Owned(linked)
            });
        let title = substitute(&self.title_template, &ctx);
        let body = substitute(&self.body_template, &ctx);
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

/// A setting as a short number: `1.25`, `0.7`, `1` — what `BirdNET-Pi` prints
/// for the same value, without the binary noise of an `f32` widened to `f64`.
fn number(value: Option<f64>) -> String {
    value.map_or_else(String::new, |v| {
        let fixed = format!("{v:.4}");
        let trimmed = fixed.trim_end_matches('0').trim_end_matches('.');
        if trimmed.is_empty() || trimmed == "-" {
            "0".to_owned()
        } else {
            trimmed.to_owned()
        }
    })
}

/// A coordinate to the six places `BirdNET-Pi` shows, or nothing.
fn coordinate(value: Option<f64>) -> String {
    value.map_or_else(String::new, |v| format!("{v:.6}"))
}

/// Perform variable substitution on a template string.
///
/// `$friendlyurl` is `BirdNET-Pi`'s: a Markdown link to the detection, not the
/// bare address of the station — a migrated template that says
/// `$friendlyurl` expects the former.
fn substitute(template: &str, ctx: &NotificationContext) -> String {
    let listen = ctx.listen_url.as_deref().unwrap_or("");
    let friendly = if listen.is_empty() {
        String::new()
    } else {
        format!("[Listen here]({listen})")
    };
    let image = ctx.image_url.as_deref().unwrap_or("");
    template
        .replace("$sciname", &ctx.sci_name)
        .replace(
            "$comname",
            ctx.shown_name.as_deref().unwrap_or(&ctx.com_name),
        )
        .replace("$confidencepct", &ctx.confidence_pct.to_string())
        .replace("$confidence", &format!("{:.2}", ctx.confidence))
        .replace("$date", &ctx.date)
        .replace("$time", &ctx.time)
        .replace("$week", &ctx.week.to_string())
        .replace("$latitude", &coordinate(ctx.latitude))
        .replace("$longitude", &coordinate(ctx.longitude))
        .replace("$reason", &ctx.reason)
        .replace("$listenurl", listen)
        .replace("$friendlyurl", &friendly)
        .replace("$flickrimage", image)
        .replace("$image", image)
        .replace("$cutoff", &number(ctx.cutoff))
        .replace("$sens", &number(ctx.sensitivity))
        .replace("$overlap", &number(ctx.overlap))
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
        self.allows(sci_name, sci_name)
    }

    /// Whether a detection passes, matching every list entry against **either**
    /// of its names, ignoring case.
    ///
    /// The settings form asks for common names ("European Robin, Great
    /// Spotted Woodpecker") and BirdNET-Pi's `APPRISE_WATCHLIST` holds common
    /// names; the command line and older configs hold scientific ones. This
    /// filter compared scientific names only, while the Apprise client read the
    /// same setting as common names only, and a detection had to pass both — so
    /// a watchlist of common names silenced every notification, the watched
    /// species included, and a watchlist of scientific names silenced Apprise.
    #[must_use]
    pub fn allows(&self, sci_name: &str, com_name: &str) -> bool {
        if names_listed(&self.exclude, sci_name, com_name) {
            return false;
        }
        self.only.is_empty() || names_listed(&self.only, sci_name, com_name)
    }
}

/// Whether `list` names either `a` or `b`, ignoring case.
pub(crate) fn names_listed<'a>(
    list: impl IntoIterator<Item = &'a String>,
    a: &str,
    b: &str,
) -> bool {
    let (a, b) = (a.trim().to_lowercase(), b.trim().to_lowercase());
    list.into_iter().any(|entry| {
        let entry = entry.trim().to_lowercase();
        entry == a || entry == b
    })
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
    pub fn should_notify(&self, sci_name: &str, counter: Option<&dyn DetectionCounter>) -> bool {
        self.should_notify_detection(sci_name, sci_name, counter)
    }

    /// [`Self::should_notify`] for a detection with both of its names, so the
    /// species lists may hold either (see [`SpeciesFilter::allows`]).
    ///
    /// `counter` must count the detections **before** this one: the trigger
    /// modes ask "is this the first today" and "has it been heard fewer than
    /// five times this week", and without a counter both modes notify on every
    /// detection, exactly like `each`.
    pub fn should_notify_detection(
        &self,
        sci_name: &str,
        com_name: &str,
        counter: Option<&dyn DetectionCounter>,
    ) -> bool {
        self.notify_reason(sci_name, com_name, counter).is_some()
    }

    /// Why a detection should notify, or `None` when it should not.
    ///
    /// The one decision [`Self::should_notify_detection`] answers as a yes or
    /// no, kept with its reason so `$reason` says what the filter actually
    /// found rather than something it was never asked. The wording is
    /// `BirdNET-Pi`'s: `detection`, `first time today`, and
    /// `only seen N times in last 7d`, where N includes this detection.
    pub fn notify_reason(
        &self,
        sci_name: &str,
        com_name: &str,
        counter: Option<&dyn DetectionCounter>,
    ) -> Option<String> {
        // Species filter check.
        if !self.species_filter.allows(sci_name, com_name) {
            return None;
        }

        // Trigger mode check.
        match self.trigger {
            TriggerMode::EachDetection => Some("detection".to_owned()),
            TriggerMode::NewSpecies => {
                let count = counter.map_or(0, |c| c.this_weeks_count_for(sci_name));
                // Notify when species has fewer than 5 detections this week.
                // The current detection has not been inserted yet, so count
                // represents previous detections.
                (count < 5)
                    .then(|| format!("only seen {} times in last 7d", count.saturating_add(1)))
            }
            TriggerMode::NewSpeciesDaily => {
                let count = counter.map_or(0, |c| c.todays_count_for(sci_name));
                // First detection of the day for this species.
                (count == 0).then(|| "first time today".to_owned())
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
            shown_name: None,
            confidence: 0.92,
            confidence_pct: 92,
            date: "2026-03-14".to_string(),
            time: "08:30:00".to_string(),
            week: 11,
            latitude: Some(51.5074),
            longitude: Some(-0.1278),
            reason: "high confidence".to_string(),
            cutoff: Some(0.7),
            sensitivity: Some(1.25),
            overlap: Some(0.0),
            listen_url: Some("http://localhost:8502/listen/123".to_string()),
            image_url: Some("http://example.com/robin.jpg".to_string()),
            station_url: Some("http://localhost:8502".to_string()),
        }
    }

    /// A context as the daemon builds it: no link of its own, so every link
    /// has to come from the template's station address.
    fn bare_context() -> NotificationContext {
        NotificationContext {
            listen_url: None,
            image_url: None,
            station_url: None,
            ..sample_context()
        }
    }

    fn render_body(template: &str, station: Option<&str>, ctx: &NotificationContext) -> String {
        NotificationTemplate::new("t".to_owned(), template.to_owned())
            .with_station_url(station.map(str::to_owned))
            .render(ctx)
            .1
    }

    /// `$latitude` and `$longitude` printed `0.000000` on every notification a
    /// production station sent: the daemon built the context with a literal 0.0.
    /// A station *with* coordinates prints them; one without prints nothing,
    /// rather than a position in the Gulf of Guinea.
    #[test]
    fn coordinates_are_the_stations_or_nothing() {
        let ctx = sample_context();
        assert_eq!(
            render_body("$latitude,$longitude", None, &ctx),
            "51.507400,-0.127800"
        );
        let none = NotificationContext {
            latitude: None,
            longitude: None,
            ..sample_context()
        };
        assert_eq!(render_body("[$latitude|$longitude]", None, &none), "[|]");
    }

    /// `$cutoff`, `$sens` and `$overlap` were replaced with nothing whatever the
    /// station's settings were.
    #[test]
    fn run_settings_render_as_short_numbers() {
        let ctx = sample_context();
        assert_eq!(
            render_body("$cutoff/$sens/$overlap", None, &ctx),
            "0.7/1.25/0"
        );
        let other = NotificationContext {
            cutoff: Some(0.5),
            sensitivity: Some(1.0),
            overlap: Some(1.5),
            ..sample_context()
        };
        assert_eq!(
            render_body("$cutoff/$sens/$overlap", None, &other),
            "0.5/1/1.5"
        );
        let unknown = NotificationContext {
            cutoff: None,
            sensitivity: None,
            overlap: None,
            ..sample_context()
        };
        assert_eq!(render_body("$cutoff/$sens/$overlap", None, &unknown), "//");
    }

    /// The links a station can build, and the ones it must not guess.
    #[test]
    fn links_come_from_the_stations_address() {
        let ctx = bare_context();
        let body = render_body(
            "$listenurl\n$friendlyurl\n$image\n$flickrimage",
            Some("http://birdnet.local:8502/"),
            &ctx,
        );
        let page = "http://birdnet.local:8502/detections/detail?date=2026-03-14&time=08%3A30%3A00&name=European%20Robin";
        let image = "http://birdnet.local:8502/api/v2/species/image/Erithacus%20rubecula/file";
        assert_eq!(
            body,
            format!("{page}\n[Listen here]({page})\n{image}\n{image}")
        );

        // No address, no link: nothing is better than a link to a guess.
        assert_eq!(
            render_body("[$listenurl][$friendlyurl][$image]", None, &ctx),
            "[][][]"
        );
        // An address that is only whitespace is no address.
        assert_eq!(render_body("[$listenurl]", Some("  "), &ctx), "[]");
    }

    /// `$friendlyurl` is `BirdNET-Pi`'s Markdown link to the detection. It was
    /// the bare address of the station, which a migrated template cannot use.
    #[test]
    fn friendly_url_is_a_markdown_link_to_the_detection() {
        let ctx = sample_context();
        assert_eq!(
            render_body("$friendlyurl", None, &ctx),
            "[Listen here](http://localhost:8502/listen/123)"
        );
    }

    /// A link the caller already set is not overwritten by the station's.
    #[test]
    fn an_explicit_link_wins_over_the_stations() {
        let ctx = sample_context();
        let body = render_body("$listenurl", Some("http://birdnet.local"), &ctx);
        assert_eq!(body, "http://localhost:8502/listen/123");
    }

    /// Characters that are not URL-safe in a species name or a time cannot
    /// change what the link points at.
    #[test]
    fn link_components_are_encoded() {
        let ctx = NotificationContext {
            com_name: "Bird & Co?x=1#y".to_owned(),
            ..bare_context()
        };
        let body = render_body("$listenurl", Some("http://s"), &ctx);
        assert!(
            body.ends_with("&name=Bird%20%26%20Co%3Fx%3D1%23y"),
            "{body}"
        );
    }

    /// A station shown in another language reads `$comname` in that language,
    /// and its links still find the detection by the name it is stored under.
    #[test]
    fn the_shown_name_is_read_and_the_stored_name_is_linked() {
        let ctx = NotificationContext {
            com_name: "Eurasian Magpie".to_owned(),
            shown_name: Some("Elster".to_owned()),
            ..bare_context()
        };
        let body = render_body("$comname | $listenurl", Some("http://s"), &ctx);
        assert!(body.starts_with("Elster | "), "{body}");
        assert!(body.ends_with("&name=Eurasian%20Magpie"), "{body}");
        // Counterpart: with no shown name, `$comname` is the stored one.
        let plain = NotificationContext {
            shown_name: None,
            ..ctx
        };
        assert!(render_body("$comname", None, &plain).starts_with("Eurasian Magpie"));
    }

    struct Counts(u64, u64);
    impl DetectionCounter for Counts {
        fn todays_count_for(&self, _: &str) -> u64 {
            self.0
        }
        fn this_weeks_count_for(&self, _: &str) -> u64 {
            self.1
        }
    }

    fn filter(trigger: TriggerMode) -> NotificationFilter {
        NotificationFilter {
            trigger,
            species_filter: SpeciesFilter::new(None, None),
        }
    }

    /// `$reason` was always empty, so a migrated `BirdNET-Pi` body that ends in
    /// `($reason)` rendered `()`. The wording is theirs.
    #[test]
    fn the_reason_names_the_trigger_that_fired() {
        let each = filter(TriggerMode::EachDetection);
        assert_eq!(
            each.notify_reason("Pica pica", "Eurasian Magpie", None)
                .as_deref(),
            Some("detection")
        );
        let daily = filter(TriggerMode::NewSpeciesDaily);
        assert_eq!(
            daily
                .notify_reason("Pica pica", "Eurasian Magpie", Some(&Counts(0, 9)))
                .as_deref(),
            Some("first time today")
        );
        assert_eq!(
            daily.notify_reason("Pica pica", "Eurasian Magpie", Some(&Counts(1, 9))),
            None,
            "the second detection of the day has no reason to notify"
        );
        let weekly = filter(TriggerMode::NewSpecies);
        // Three earlier detections plus this one: "seen 4 times", as
        // BirdNET-Pi counts after the insert.
        assert_eq!(
            weekly
                .notify_reason("Pica pica", "Eurasian Magpie", Some(&Counts(9, 3)))
                .as_deref(),
            Some("only seen 4 times in last 7d")
        );
        assert_eq!(
            weekly.notify_reason("Pica pica", "Eurasian Magpie", Some(&Counts(9, 5))),
            None
        );
    }

    /// The reason is the yes/no answer's own explanation, not a second opinion:
    /// an excluded species gets neither.
    #[test]
    fn an_excluded_species_has_no_reason() {
        let f = NotificationFilter {
            trigger: TriggerMode::EachDetection,
            species_filter: SpeciesFilter::new(Some("Eurasian Magpie"), None),
        };
        assert_eq!(f.notify_reason("Pica pica", "Eurasian Magpie", None), None);
        assert!(!f.should_notify_detection("Pica pica", "Eurasian Magpie", None));
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
            "A $comname ($sciname) was detected at $time with $confidencepct% confidence."
                .to_string(),
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
        let counter = MockCounter {
            daily: 100,
            weekly: 500,
        };
        assert!(filter.should_notify("Any", Some(&counter)));
    }

    #[test]
    fn trigger_new_species_respects_weekly_count() {
        let filter = NotificationFilter {
            trigger: TriggerMode::NewSpecies,
            species_filter: SpeciesFilter::new(None, None),
        };
        let counter_low = MockCounter {
            daily: 0,
            weekly: 3,
        };
        assert!(filter.should_notify("Robin", Some(&counter_low)));

        let counter_high = MockCounter {
            daily: 0,
            weekly: 5,
        };
        assert!(!filter.should_notify("Robin", Some(&counter_high)));
    }

    #[test]
    fn trigger_new_species_daily_respects_daily_count() {
        let filter = NotificationFilter {
            trigger: TriggerMode::NewSpeciesDaily,
            species_filter: SpeciesFilter::new(None, None),
        };
        let counter_zero = MockCounter {
            daily: 0,
            weekly: 50,
        };
        assert!(filter.should_notify("Robin", Some(&counter_zero)));

        let counter_nonzero = MockCounter {
            daily: 1,
            weekly: 50,
        };
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
