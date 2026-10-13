//! Which blocks the Today page shows, and in what order (`G-22`).
//!
//! A station on a kitchen tablet or a visitor-centre wall needs a different
//! page from one an operator is debugging, and the blocks of the Today page are
//! already independent HTMX partials. This decides which of them are rendered
//! and in what order; nothing about any block's own content changes.
//!
//! # Stored as one setting
//!
//! `today_layout` is a comma-separated list of block ids in display order, a
//! hidden block prefixed with `-`: `daystrip,hero,-nudge,log,top-species,best,
//! -station`. Parsing is forgiving in one direction only — an unknown id is
//! dropped and a block the value does not mention is appended, shown, in its
//! default position — so a value written by an older release still renders
//! every block a newer one adds, and nothing an operator wrote can remove a
//! block they never asked to hide.
//!
//! # Order is the document's order
//!
//! Blocks are reassembled in the DOM, not reordered with CSS `order`, so the
//! reading order, the tab order and what a screen reader announces all follow
//! what the page shows (WCAG 1.3.2, 2.4.3).
//!
//! # Three groups
//!
//! The page is a stack of full-width blocks above a two-column grid: the log
//! on the left, a rail of cards on the right. A top block moves among the top
//! blocks and a rail card within the rail; the log keeps its place and can
//! only be shown or hidden. Moving a card into the stack would need a layout
//! the page does not have. When the log or every rail card is hidden, the
//! other takes the full width.
//!
//! # Not on a brand-new station
//!
//! The first-run page carries the setup checklist in the hero. A layout that
//! hid it would hide the station's own instructions for getting started, so a
//! station with no detections yet renders the default.

/// The setting that holds the layout.
pub const SETTING_KEY: &str = "today_layout";

/// One block of the Today page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Block {
    /// The phrase about today and the live-signal panel.
    Hero,
    /// The one-line suggestion under the hero.
    Nudge,
    /// The 24-hour strip.
    DayStrip,
    /// The unified detection log.
    Log,
    /// Rail: today's top species.
    TopSpecies,
    /// Rail: today's highest-confidence recordings.
    Best,
    /// Rail: the station's health line and the "looking back" links.
    Station,
}

impl Block {
    /// Every block, in the default order. Main-column blocks first.
    pub const ALL: [Self; 7] = [
        Self::Hero,
        Self::Nudge,
        Self::DayStrip,
        Self::Log,
        Self::TopSpecies,
        Self::Best,
        Self::Station,
    ];

    /// The id written in the setting and in the template's markers.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Hero => "hero",
            Self::Nudge => "nudge",
            Self::DayStrip => "daystrip",
            Self::Log => "log",
            Self::TopSpecies => "top-species",
            Self::Best => "best",
            Self::Station => "station",
        }
    }

    /// The name shown on the settings page.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Hero => "Today's headline and live signal",
            Self::Nudge => "Suggestion line",
            Self::DayStrip => "Day strip (24 hours)",
            Self::Log => "Detection log",
            Self::TopSpecies => "Top species",
            Self::Best => "Best recordings",
            Self::Station => "Your station and looking back",
        }
    }

    /// Which group the block moves within.
    #[must_use]
    pub const fn group(self) -> Group {
        match self {
            Self::Hero | Self::Nudge | Self::DayStrip => Group::Top,
            Self::Log => Group::Log,
            Self::TopSpecies | Self::Best | Self::Station => Group::Rail,
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.id() == id)
    }
}

/// Where on the page a block can move. See the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// The full-width stack at the top.
    Top,
    /// The log, alone in its group: it can be hidden but not moved.
    Log,
    /// The cards beside the log.
    Rail,
}

/// A block and whether it is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    /// Which block.
    pub block: Block,
    /// Whether it is rendered.
    pub shown: bool,
}

/// The order and visibility of every block — each exactly once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    slots: Vec<Slot>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            slots: Block::ALL
                .into_iter()
                .map(|block| Slot { block, shown: true })
                .collect(),
        }
    }
}

impl Layout {
    /// Read a stored value. See the module docs for what is forgiven.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        let mut slots: Vec<Slot> = Vec::with_capacity(Block::ALL.len());
        for raw in value.split(',') {
            let raw = raw.trim();
            let (shown, id) = raw.strip_prefix('-').map_or((true, raw), |id| (false, id));
            let Some(block) = Block::from_id(id.trim()) else {
                continue;
            };
            if !slots.iter().any(|s| s.block == block) {
                slots.push(Slot { block, shown });
            }
        }
        for (default_index, block) in Block::ALL.into_iter().enumerate() {
            if slots.iter().any(|s| s.block == block) {
                continue;
            }
            // In its default position among the blocks already placed: after
            // the last block that precedes it by default.
            let at = Block::ALL[..default_index]
                .iter()
                .filter_map(|prev| slots.iter().position(|s| s.block == *prev))
                .max()
                .map_or(0, |i| i + 1);
            slots.insert(at, Slot { block, shown: true });
        }
        Self { slots }
    }

    /// The value to store.
    #[must_use]
    pub fn to_setting(&self) -> String {
        self.slots
            .iter()
            .map(|s| {
                if s.shown {
                    s.block.id().to_owned()
                } else {
                    format!("-{}", s.block.id())
                }
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Every slot, in order.
    #[must_use]
    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    /// Show or hide `block`.
    pub fn set_shown(&mut self, block: Block, shown: bool) {
        if let Some(slot) = self.slots.iter_mut().find(|s| s.block == block) {
            slot.shown = shown;
        }
    }

    /// Move `block` one place earlier (`up`) or later within its group.
    /// At the edge of its group it stays where it is.
    pub fn nudge(&mut self, block: Block, up: bool) {
        let Some(i) = self.slots.iter().position(|s| s.block == block) else {
            return;
        };
        let group = block.group();
        let neighbour = if up {
            self.slots[..i]
                .iter()
                .rposition(|s| s.block.group() == group)
        } else {
            self.slots[i + 1..]
                .iter()
                .position(|s| s.block.group() == group)
                .map(|j| i + 1 + j)
        };
        if let Some(j) = neighbour {
            self.slots.swap(i, j);
        }
    }

    fn shown(&self, block: Block) -> bool {
        self.slots.iter().any(|s| s.block == block && s.shown)
    }

    fn ordered(&self, group: Group) -> impl Iterator<Item = Block> + '_ {
        self.slots
            .iter()
            .filter(move |s| s.shown && s.block.group() == group)
            .map(|s| s.block)
    }

    /// Reassemble `template` in this layout's order.
    ///
    /// The template marks each block with `<!--block:ID-->` … `<!--/block:ID-->`
    /// and the two movable regions with `<!--top-->`…`<!--/top-->` and
    /// `<!--rail-->`…`<!--/rail-->`; `{{work2_solo}}` receives the class that
    /// lets the log or the rail take the full width. Inside a region, text
    /// between blocks is dropped with nothing to separate; outside one, every
    /// byte keeps its place.
    ///
    /// # Errors
    ///
    /// The first marker missing from the template. The template is compiled
    /// in, so this is a defect in this crate, which the tests catch; the
    /// caller then renders the default.
    pub fn apply(&self, template: &str) -> Result<String, String> {
        let (before_top, top, after_top) = region(template, "top")?;
        let top = self.reassemble(top, Group::Top)?;

        let (log_open, log_close) = markers(Block::Log);
        let ls = after_top.find(&log_open).ok_or_else(|| log_open.clone())?;
        let le = after_top[ls..]
            .find(&log_close)
            .ok_or_else(|| log_close.clone())?
            + ls;
        let log_body = &after_top[ls + log_open.len()..le];
        let after_top = format!(
            "{}{}{}",
            &after_top[..ls],
            if self.shown(Block::Log) { log_body } else { "" },
            &after_top[le + log_close.len()..]
        );

        let (before_rail, rail, after_rail) = region(&after_top, "rail")?;
        let rail = self.reassemble(rail, Group::Rail)?;

        // The log and the rail share a two-column grid; when one of them is
        // gone the other takes the whole width.
        let has_log = self.shown(Block::Log);
        let has_rail = self.ordered(Group::Rail).next().is_some();
        let solo = match (has_log, has_rail) {
            (true, true) => "",
            (true, false) => " work2--log-only",
            (false, true) => " work2--rail-only",
            (false, false) => " work2--empty",
        };
        Ok(format!("{before_top}{top}{before_rail}{rail}{after_rail}")
            .replace("{{work2_solo}}", solo))
    }

    /// The shown blocks of `group`, in order, cut from `region_text`.
    fn reassemble(&self, region_text: &str, group: Group) -> Result<String, String> {
        let mut bodies: Vec<(Block, &str)> = Vec::new();
        for block in Block::ALL.into_iter().filter(|b| b.group() == group) {
            let (open, close) = markers(block);
            let start = region_text.find(&open).ok_or_else(|| open.clone())? + open.len();
            let end = region_text[start..]
                .find(&close)
                .ok_or_else(|| close.clone())?
                + start;
            bodies.push((block, &region_text[start..end]));
        }
        Ok(self
            .ordered(group)
            .filter_map(|block| {
                bodies
                    .iter()
                    .find(|(b, _)| *b == block)
                    .map(|(_, body)| *body)
            })
            .collect())
    }
}

fn markers(block: Block) -> (String, String) {
    (
        format!("<!--block:{}-->", block.id()),
        format!("<!--/block:{}-->", block.id()),
    )
}

/// `(before, inside, after)` for `<!--name-->…<!--/name-->`.
fn region<'a>(text: &'a str, name: &str) -> Result<(&'a str, &'a str, &'a str), String> {
    let open = format!("<!--{name}-->");
    let close = format!("<!--/{name}-->");
    let start = text.find(&open).ok_or_else(|| open.clone())?;
    let inner = start + open.len();
    let end = text[inner..].find(&close).ok_or_else(|| close.clone())? + inner;
    Ok((
        &text[..start],
        &text[inner..end],
        &text[end + close.len()..],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(layout: &Layout) -> String {
        layout.to_setting()
    }

    #[test]
    fn the_default_shows_every_block_in_page_order() {
        assert_eq!(
            ids(&Layout::default()),
            "hero,nudge,daystrip,log,top-species,best,station"
        );
        assert_eq!(Layout::parse(""), Layout::default());
    }

    #[test]
    fn a_stored_layout_round_trips() {
        let v = "daystrip,hero,-nudge,log,best,-station,top-species";
        assert_eq!(ids(&Layout::parse(v)), v);
    }

    /// An older value that never heard of a block still renders it, in its
    /// default place; an unknown id and a repeat are dropped.
    #[test]
    fn a_partial_or_foreign_value_is_completed_not_trusted() {
        assert_eq!(
            ids(&Layout::parse("log, -hero, video-embed, log")),
            "log,-hero,nudge,daystrip,top-species,best,station"
        );
    }

    #[test]
    fn moving_stays_inside_the_group() {
        let mut l = Layout::default();
        l.nudge(Block::TopSpecies, true); // first in the rail: stays
        assert_eq!(ids(&l), ids(&Layout::default()));
        l.nudge(Block::Log, false); // alone in its group: stays
        l.nudge(Block::Log, true);
        assert_eq!(ids(&l), ids(&Layout::default()));
        l.nudge(Block::DayStrip, true);
        l.nudge(Block::Station, true);
        l.set_shown(Block::Nudge, false);
        assert_eq!(ids(&l), "hero,daystrip,-nudge,log,top-species,station,best");
    }

    const TEMPLATE: &str = "<head>\
        <!--top--><!--block:hero-->H<!--/block:hero--><!--block:nudge-->N<!--/block:nudge-->\
        <!--block:daystrip-->D<!--/block:daystrip--><!--/top-->\
        <div class=\"work2{{work2_solo}}\"><!--block:log-->L<!--/block:log-->\
        <aside><!--rail--><!--block:top-species-->T<!--/block:top-species-->\
        <!--block:best-->B<!--/block:best--><!--block:station-->S<!--/block:station-->\
        <!--/rail--></aside></div><end>";

    #[test]
    fn the_page_is_reassembled_in_layout_order() {
        let l = Layout::parse("daystrip,-nudge,hero,log,station,top-species,-best");
        let out = l.apply(TEMPLATE).expect("markers present");
        assert_eq!(
            out,
            "<head>DH<div class=\"work2\">L<aside>ST</aside></div><end>"
        );
    }

    /// The counterpart: the default layout reproduces the template's own
    /// order exactly, so reassembly is not a reordering of its own.
    #[test]
    fn the_default_layout_is_the_template_as_written() {
        let out = Layout::default().apply(TEMPLATE).expect("markers present");
        assert_eq!(
            out,
            "<head>HND<div class=\"work2\">L<aside>TBS</aside></div><end>"
        );
    }

    #[test]
    fn the_survivor_of_log_and_rail_takes_the_width() {
        let no_rail = Layout::parse("-top-species,-best,-station");
        assert!(
            no_rail
                .apply(TEMPLATE)
                .unwrap()
                .contains("work2 work2--log-only")
        );
        let no_log = Layout::parse("-log");
        let out = no_log.apply(TEMPLATE).unwrap();
        assert!(out.contains("work2 work2--rail-only"), "{out}");
        assert!(!out.contains('L'), "the log is gone: {out}");
    }

    #[test]
    fn a_missing_marker_is_named() {
        let broken = TEMPLATE.replace("<!--/block:best-->", "");
        assert_eq!(
            Layout::default().apply(&broken),
            Err("<!--/block:best-->".to_owned())
        );
    }
}
