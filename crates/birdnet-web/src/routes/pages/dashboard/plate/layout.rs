//! Where each species goes on the kiosk plate.
//!
//! A pure function from "how many times each species was heard" to ovals on a
//! sheet: no I/O, no clock, no randomness, so the same birds give the same
//! plate and a refresh does not shuffle anything. Everything the renderer needs
//! to know about geometry is decided here, and everything that can go wrong
//! with geometry (two pictures on each other, a caption off the sheet, the
//! busiest bird smaller than a rare one) is a property the tests below hold.
//!
//! # Sizing
//!
//! A picture's *area* grows with the square root of the species' count, so its
//! width grows with the fourth root, and it is never narrower than
//! [`MIN_WIDTH_SHARE`] of the widest. That is a hierarchy, not a measurement:
//! the plate is there to be looked at, the exact counts are printed in its key,
//! and a station that heard one bird 1.6 million times and another once must
//! not draw the second at one pixel. The rule is printed on the plate.
//!
//! # Placement
//!
//! Largest first. The first goes to the middle of the free area and each next
//! one takes the first place along a golden-angle spiral out from there where
//! it, and the caption under it, touch nothing already placed. If something
//! does not fit, everything shrinks by [`SHRINK`] and the placement starts
//! again; a plate with more birds than room draws them smaller, and at a floor
//! it stops drawing the rarest and lets the key carry them.
//!
//! Idea after fugleramme (Arne Giacomo Munthe-Kaas, MIT): a collage of the
//! birds heard, the bigger toward the middle. No code or artwork is shared.

/// The narrowest a picture may be, as a share of the widest.
pub const MIN_WIDTH_SHARE: f64 = 0.38;
/// Height of an oval as a share of its width.
pub const ASPECT: f64 = 0.68;
/// The widest a caption box is asked to be. A name that needs more is cut to this.
pub const CAPTION_MAX_W: f64 = 300.0;
/// Room under each oval for the Latin name and the common name.
pub const CAPTION_H: f64 = 46.0;
/// The narrowest a caption box is, so a short name still keeps its neighbours off.
pub const CAPTION_MIN_W: f64 = 150.0;
/// Clear space kept between one figure and the next.
pub const GAP: f64 = 14.0;
/// Each shrink step when the birds do not fit.
const SHRINK: f64 = 0.93;
/// The smallest the widest picture may become, as a share of its first guess.
const FLOOR: f64 = 0.22;
/// An upper bound on shrink steps. `SHRINK^21` is already below [`FLOOR`], so the bound is never
/// the thing that stops the loop; it exists so the loop is visibly finite.
const MAX_SHRINKS: i32 = 40;
/// Spiral steps tried per figure before giving up on a scale.
const STEPS: u32 = 4000;
/// Radius growth per spiral step.
const STEP_R: f64 = 11.0;
/// The golden angle, in radians.
const GOLDEN: f64 = 2.399_963_229_728_653;

/// A rectangle: left, top, width, height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
}

impl Rect {
    /// Whether two rectangles come closer than `gap` to each other.
    fn touches(self, other: Self, gap: f64) -> bool {
        self.x < other.x + other.w + gap
            && other.x < self.x + self.w + gap
            && self.y < other.y + other.h + gap
            && other.y < self.y + self.h + gap
    }

    /// Whether `self` lies wholly inside `outer`.
    fn inside(self, outer: Self) -> bool {
        self.x >= outer.x
            && self.y >= outer.y
            && self.x + self.w <= outer.x + outer.w
            && self.y + self.h <= outer.y + outer.h
    }
}

/// One placed picture. `cx`, `cy` are the centre of the oval; its caption sits
/// directly beneath it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Index into the counts the layout was given.
    pub index: usize,
    /// Centre, horizontal.
    pub cx: f64,
    /// Centre, vertical.
    pub cy: f64,
    /// Oval width.
    pub w: f64,
    /// Oval height.
    pub h: f64,
    /// Width of the caption box beneath the oval, at least the oval's own: the room the two
    /// names need, so a long common name is not cut where a neighbour is far away.
    pub cap_w: f64,
}

impl Placement {
    /// The oval's bounding box.
    #[must_use]
    pub fn oval(&self) -> Rect {
        Rect {
            x: self.cx - self.w / 2.0,
            y: self.cy - self.h / 2.0,
            w: self.w,
            h: self.h,
        }
    }

    /// The caption's box, centred under the oval.
    #[must_use]
    pub fn caption(&self) -> Rect {
        let w = self.w.max(self.cap_w).max(CAPTION_MIN_W);
        Rect {
            x: self.cx - w / 2.0,
            y: self.cy + self.h / 2.0,
            w,
            h: CAPTION_H,
        }
    }

    /// Everything the figure occupies: the oval and its caption.
    #[must_use]
    pub fn occupied(&self) -> [Rect; 2] {
        [self.oval(), self.caption()]
    }
}

/// How wide a picture is, in `0.38..=1.0` of the widest, for `count` of `max`.
///
/// Monotone in `count`; a count of zero is treated as one.
#[must_use]
pub fn width_share(count: u64, max: u64) -> f64 {
    let max = max.max(1);
    let count = count.clamp(1, max);
    // u64 to f64 can round above 2^53; a count of that size is not a count of
    // birds, and the ratio below only has to be monotone.
    #[allow(clippy::cast_precision_loss)]
    let ratio = count as f64 / max as f64;
    ratio.powf(0.25).clamp(MIN_WIDTH_SHARE, 1.0)
}

/// Place the species heard `counts[i]` times each inside `free`, each with a caption box
/// `caption_w[i]` wide (at least its oval's width).
///
/// Returns one [`Placement`] per species that fit, in input order of `index`.
/// A species that cannot be placed even at the smallest size is left out, and
/// it is always among the rarest, because placement is largest first.
/// `seed` turns the spiral, so each day's plate is a little different and one
/// day's plate is stable.
#[must_use]
pub fn place(counts: &[u64], caption_w: &[f64], free: Rect, seed: u32) -> Vec<Placement> {
    if counts.is_empty() || free.w <= 0.0 || free.h <= 0.0 {
        return Vec::new();
    }
    // Largest first; ties by index so the result does not depend on sort stability.
    let mut order: Vec<usize> = (0..counts.len()).collect();
    order.sort_by(|&a, &b| counts[b].cmp(&counts[a]).then(a.cmp(&b)));
    let max = counts[order[0]];

    let first_guess = (free.w * 0.42)
        .min((free.h - CAPTION_H) / ASPECT * 0.62)
        .max(40.0);
    let mut keep = order.len();
    loop {
        for step in 0..MAX_SHRINKS {
            let scale = SHRINK.powi(step);
            if scale < FLOOR {
                break;
            }
            if let Some(done) = try_scale(
                &order[..keep],
                counts,
                caption_w,
                max,
                free,
                first_guess * scale,
                seed,
            ) {
                return centred(done, free);
            }
        }
        // Even the floor does not hold this many: drop the rarest and go again.
        if keep <= 1 {
            return Vec::new();
        }
        keep -= 1;
    }
}

/// One attempt at a given widest-picture width.
fn try_scale(
    order: &[usize],
    counts: &[u64],
    caption_w: &[f64],
    max: u64,
    free: Rect,
    widest: f64,
    seed: u32,
) -> Option<Vec<Placement>> {
    let aspect = (free.w / free.h).sqrt();
    let (ax, ay) = (aspect, 1.0 / aspect);
    let (mx, my) = (free.x + free.w / 2.0, free.y + free.h / 2.0);
    let turn = f64::from(seed % 360).to_radians();
    let mut placed: Vec<Placement> = Vec::with_capacity(order.len());
    for &i in order {
        let w = widest * width_share(counts[i], max);
        let h = w * ASPECT;
        let mut fit = None;
        for t in 0..STEPS {
            let r = STEP_R * f64::from(t).sqrt();
            let a = GOLDEN * f64::from(t) + turn;
            let cand = Placement {
                index: i,
                cx: mx + r * a.cos() * ax,
                cy: my + r * a.sin() * ay - CAPTION_H / 2.0,
                w,
                h,
                cap_w: caption_w[i],
            };
            let boxes = cand.occupied();
            if !boxes.iter().all(|b| b.inside(free)) {
                continue;
            }
            let clear = placed.iter().all(|p| {
                let theirs = p.occupied();
                boxes
                    .iter()
                    .all(|b| theirs.iter().all(|o| !b.touches(*o, GAP)))
            });
            if clear {
                fit = Some(cand);
                break;
            }
        }
        placed.push(fit?);
    }
    Some(placed)
}

/// Slide the whole group so its bounding box is centred in `free`.
fn centred(mut placed: Vec<Placement>, free: Rect) -> Vec<Placement> {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in &placed {
        for b in p.occupied() {
            x0 = x0.min(b.x);
            y0 = y0.min(b.y);
            x1 = x1.max(b.x + b.w);
            y1 = y1.max(b.y + b.h);
        }
    }
    let dx = free.x + free.w / 2.0 - f64::midpoint(x0, x1);
    let dy = free.y + free.h / 2.0 - f64::midpoint(y0, y1);
    // The group already fits; sliding by the centring offset cannot leave the
    // free area because the group's bounding box is no larger than it.
    for p in &mut placed {
        p.cx += dx;
        p.cy += dy;
    }
    placed.sort_by_key(|p| p.index);
    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHEET: Rect = Rect {
        x: 90.0,
        y: 250.0,
        w: 1420.0,
        h: 560.0,
    };
    /// The free area `render::Sheet::of` gives a 9:16 screen with twelve species: a 1600-unit
    /// sheet 2844 high, less the plate-mark and padding, a 208-unit title block, a two-column
    /// key of six rows and the foot.
    const PHONE: Rect = Rect {
        x: 90.0,
        y: 242.0,
        w: 1420.0,
        h: 2350.0,
    };

    /// Every caption as wide as the floor: the layout as it was before captions had their own
    /// widths.
    fn place_flat(counts: &[u64], free: Rect, seed: u32) -> Vec<Placement> {
        place(counts, &vec![CAPTION_MIN_W; counts.len()], free, seed)
    }

    fn assert_clean(ps: &[Placement], free: Rect) {
        for p in ps {
            for b in p.occupied() {
                assert!(b.inside(free), "figure {} leaves the sheet: {b:?}", p.index);
            }
        }
        for (i, a) in ps.iter().enumerate() {
            for b in &ps[i + 1..] {
                for x in a.occupied() {
                    for y in b.occupied() {
                        assert!(
                            !x.touches(y, 0.0),
                            "figures {} and {} overlap: {x:?} {y:?}",
                            a.index,
                            b.index
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn nothing_heard_places_nothing() {
        assert!(place_flat(&[], SHEET, 7).is_empty());
    }

    #[test]
    fn one_bird_sits_in_the_middle() {
        let ps = place_flat(&[5], SHEET, 7);
        assert_eq!(ps.len(), 1);
        let (mx, my) = (SHEET.x + SHEET.w / 2.0, SHEET.y + SHEET.h / 2.0);
        let b = &ps[0];
        let whole_y = b.oval().y + (b.caption().y + b.caption().h - b.oval().y) / 2.0;
        assert!((b.cx - mx).abs() < 1.0, "centred across: {} vs {mx}", b.cx);
        assert!(
            (whole_y - my).abs() < 1.0,
            "figure plus caption centred down"
        );
    }

    #[test]
    fn a_busy_garden_fits_and_nothing_overlaps() {
        let counts: Vec<u64> = (1..=24).map(|n| 400 / n).collect();
        let ps = place_flat(&counts, SHEET, 11);
        assert_eq!(ps.len(), 24, "all twenty-four fit at some size");
        assert_clean(&ps, SHEET);
    }

    #[test]
    fn a_tall_phone_sheet_is_just_as_clean() {
        let counts: Vec<u64> = (1..=12).map(|n| 90 / n + 1).collect();
        let ps = place_flat(&counts, PHONE, 3);
        assert_eq!(ps.len(), 12);
        assert_clean(&ps, PHONE);
    }

    #[test]
    fn the_busiest_bird_is_never_smaller_than_a_rarer_one() {
        let counts = [3, 90, 14, 14, 1, 600, 28];
        let ps = place_flat(&counts, SHEET, 5);
        for a in &ps {
            for b in &ps {
                if counts[a.index] > counts[b.index] {
                    assert!(
                        a.w >= b.w - 1e-9,
                        "{} heard more than {} but drawn narrower",
                        a.index,
                        b.index
                    );
                }
            }
        }
    }

    #[test]
    fn equal_counts_are_drawn_equal() {
        let ps = place_flat(&[7, 7, 7, 7], SHEET, 9);
        assert!(ps.windows(2).all(|w| (w[0].w - w[1].w).abs() < 1e-9));
    }

    #[test]
    fn a_million_to_one_still_draws_the_rare_bird_legibly() {
        let ps = place_flat(&[1_600_000, 1], SHEET, 2);
        assert_eq!(ps.len(), 2);
        let ratio = ps[1].w / ps[0].w;
        assert!(
            (ratio - MIN_WIDTH_SHARE).abs() < 1e-9,
            "floor, not 1/1000: {ratio}"
        );
    }

    #[test]
    fn the_same_birds_give_the_same_plate() {
        let counts = [12, 40, 7, 7, 3];
        assert_eq!(
            place_flat(&counts, SHEET, 21),
            place_flat(&counts, SHEET, 21)
        );
    }

    #[test]
    fn too_many_birds_drop_the_rarest_and_never_overlap() {
        let counts: Vec<u64> = (1..=200).rev().collect();
        let tiny = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 300.0,
        };
        let ps = place_flat(&counts, tiny, 1);
        assert!(!ps.is_empty() && ps.len() < 200);
        assert_clean(&ps, tiny);
        // What was dropped is the rarest: every kept bird was heard at least
        // as often as every dropped one.
        let kept_min = ps.iter().map(|p| counts[p.index]).min().unwrap();
        let dropped_max = (0..counts.len())
            .filter(|i| ps.iter().all(|p| p.index != *i))
            .map(|i| counts[i])
            .max()
            .unwrap();
        assert!(kept_min >= dropped_max);
    }

    #[test]
    fn width_share_is_monotone_and_bounded() {
        let mut last = 0.0;
        for c in [0_u64, 1, 2, 10, 100, 10_000, 1_000_000] {
            let s = width_share(c, 1_000_000);
            assert!((MIN_WIDTH_SHARE..=1.0).contains(&s));
            assert!(s >= last);
            last = s;
        }
        assert!((width_share(1_000_000, 1_000_000) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_sheet_with_no_room_places_nothing_instead_of_looping() {
        let none = Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 100.0,
        };
        assert!(place_flat(&[3, 2, 1], none, 4).is_empty());
    }

    #[test]
    fn a_long_name_gets_the_room_it_asks_for_and_nothing_overlaps_it() {
        let counts = [40, 30, 20, 10, 5, 5, 3, 2];
        let mut widths = vec![CAPTION_MIN_W; counts.len()];
        widths[2] = CAPTION_MAX_W; // "RUBY-THROATED HUMMINGBIRD"
        let ps = place(&counts, &widths, SHEET, 6);
        assert_eq!(ps.len(), counts.len());
        let long = ps.iter().find(|p| p.index == 2).unwrap();
        assert!(
            long.caption().w >= CAPTION_MAX_W - 1e-9,
            "{:?}",
            long.caption()
        );
        assert_clean(&ps, SHEET);
    }

    #[test]
    fn a_caption_is_never_narrower_than_its_oval_or_the_floor() {
        let wide_oval = Placement {
            index: 0,
            cx: 500.0,
            cy: 300.0,
            w: 400.0,
            h: 272.0,
            cap_w: 10.0,
        };
        assert!((wide_oval.caption().w - 400.0).abs() < 1e-9);
        let small_oval = Placement {
            index: 0,
            cx: 500.0,
            cy: 300.0,
            w: 80.0,
            h: 54.0,
            cap_w: 10.0,
        };
        assert!((small_oval.caption().w - CAPTION_MIN_W).abs() < 1e-9);
    }
}
