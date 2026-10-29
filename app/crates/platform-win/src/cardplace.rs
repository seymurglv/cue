//! Where the "Ask" card goes (spec 2 §7): at the selection, its arrow pointing at the selection's start — from under it, or
//! from above it when there is no room below — inside the work area of the selection's monitor. Pure: physical px in, the
//! card window's rect out.
use crate::placement::{Monitor, PxRect};

/// The card's sizes, logical px (the approved design, 04-answer): the same numbers as `.ansC` in
/// `app/ui/src/design/answer.css` (a test keeps them equal).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardMetrics {
    pub width: f64,
    /// The tallest the card gets; less where the room is short.
    pub max_height: f64,
    /// The arrow's height over (or under) the card.
    pub arrow: f64,
    /// From the selection to the arrow's tip.
    pub gap: f64,
    /// Transparent room for the shadow around the card in its window.
    pub pad: f64,
    /// From the work area's edge to the card.
    pub margin: f64,
    /// The arrow points at most this far into the selection's first line (at its middle when the line is shorter).
    pub lead: f64,
    /// The arrow keeps this far from the card's rounded corners.
    pub arrow_inset: f64,
}

pub const CARD: CardMetrics = CardMetrics {
    width: 390.0,
    max_height: 300.0,
    arrow: 6.0,
    gap: 4.0,
    pad: 24.0,
    margin: 8.0,
    lead: 140.0,
    arrow_inset: 18.0,
};
/// The card's left edge is this far left of its arrow (the design).
pub const ARROW_FROM_LEFT: f64 = 60.0;
/// A card shorter than this shows too little: the answer goes to the tray instead.
pub const MIN_CARD: f64 = 120.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardPlace {
    /// The card window, physical px, at the size it has on this monitor.
    pub rect: PxRect,
    /// The card is under the selection, its arrow pointing up; otherwise above it.
    pub below: bool,
    /// The arrow's x from the card's left edge, logical px.
    pub arrow_x: f64,
    /// How tall the card may grow here, logical px.
    pub max_height: f64,
}

/// The card window's logical size: the tallest card with its arrow, and the shadow room around it.
pub fn window_size(m: CardMetrics) -> (f64, f64) {
    (m.width + 2.0 * m.pad, m.max_height + m.arrow + 2.0 * m.pad)
}

/// The card for a selection whose first line is `first` and whose lines all fit in `all` (physical px), on `mon`. `None` —
/// the selection is off the work area, or there is no room for a useful card: the tray then (spec 2 §7).
pub fn card_place(first: PxRect, all: PxRect, mon: Monitor, m: CardMetrics) -> Option<CardPlace> {
    let (work, s) = (mon.work, mon.scale);
    let px = |v: f64| (v * s).round() as i32;
    // The card and both margins, each rounded as the clamp below rounds it: a work area that holds them never makes that
    // clamp's bounds cross.
    if !all.intersects(&work) || work.width() < px(m.width) + 2 * px(m.margin) {
        return None;
    }
    // Across: the arrow into the first line, the card around it, inside the work area.
    let edge = px(m.margin + m.arrow_inset);
    let arrow = (first.left + px(m.lead).min(first.width() / 2)).clamp(work.left + edge, work.right - edge);
    let card_left = (arrow - px(ARROW_FROM_LEFT)).clamp(work.left + px(m.margin), work.right - px(m.margin) - px(m.width));
    let arrow_x = (f64::from(arrow - card_left) / s).clamp(m.arrow_inset, m.width - m.arrow_inset);
    // Up and down: under the selection when the tallest card fits there, or when there is no more room above.
    let reach = px(m.gap + m.arrow);
    let room_below = work.bottom - px(m.margin) - (all.bottom + reach);
    let room_above = all.top - reach - (work.top + px(m.margin));
    let below = room_below >= px(m.max_height) || room_below >= room_above;
    let room = f64::from(if below { room_below } else { room_above }) / s;
    if room < MIN_CARD {
        return None;
    }
    let (w, h) = window_size(m);
    let (w, h) = (px(w), px(h));
    // The arrow's tip is `pad` from the window's edge (`.ansC` sits at pad + arrow).
    let left = card_left - px(m.pad);
    let top = if below { all.bottom + px(m.gap) - px(m.pad) } else { all.top - px(m.gap) + px(m.pad) - h };
    let rect = PxRect { left, top, right: left + w, bottom: top + h };
    Some(CardPlace { rect, below, arrow_x, max_height: room.min(m.max_height) })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FHD: Monitor = Monitor { work: PxRect { left: 0, top: 0, right: 1920, bottom: 1032 }, scale: 1.0 };

    fn r(left: i32, top: i32, right: i32, bottom: i32) -> PxRect {
        PxRect { left, top, right, bottom }
    }

    fn line(left: i32, top: i32, right: i32, bottom: i32) -> (PxRect, PxRect) {
        (r(left, top, right, bottom), r(left, top, right, bottom))
    }

    #[test]
    fn the_window_holds_the_tallest_card_with_its_arrow_and_shadow() {
        assert_eq!(window_size(CARD), (438.0, 354.0));
    }

    #[test]
    fn under_the_selection_the_arrow_points_up_at_its_start() {
        let (first, all) = line(500, 300, 900, 320);
        let p = card_place(first, all, FHD, CARD).unwrap();
        assert_eq!(p, CardPlace { rect: r(556, 300, 994, 654), below: true, arrow_x: 60.0, max_height: 300.0 });
    }

    #[test]
    fn without_room_below_the_card_goes_above() {
        let (first, all) = line(500, 900, 900, 920);
        let p = card_place(first, all, FHD, CARD).unwrap();
        assert_eq!(p, CardPlace { rect: r(556, 566, 994, 920), below: false, arrow_x: 60.0, max_height: 300.0 });
    }

    #[test]
    fn the_arrow_takes_the_first_line_the_card_clears_all_lines() {
        let p = card_place(r(700, 300, 1000, 320), r(100, 300, 1000, 380), FHD, CARD).unwrap();
        assert_eq!((p.rect, p.arrow_x), (r(756, 360, 1194, 714), 60.0));
    }

    #[test]
    fn a_short_first_line_gets_the_arrow_at_its_middle() {
        let (first, all) = line(500, 300, 560, 320);
        assert_eq!(card_place(first, all, FHD, CARD).unwrap().rect.left, 446);
    }

    #[test]
    fn at_the_edges_the_card_stays_inside_and_the_arrow_moves() {
        let (first, all) = line(1800, 300, 1900, 320);
        let p = card_place(first, all, FHD, CARD).unwrap();
        assert_eq!((p.rect.left, p.arrow_x), (1498, 328.0), "the card at 1522, 8 px from the right edge");
        let (first, all) = line(0, 300, 30, 320);
        let p = card_place(first, all, FHD, CARD).unwrap();
        assert_eq!((p.rect.left, p.arrow_x), (-16, 18.0), "the card 8 px from the left edge, the arrow off the corner");
    }

    #[test]
    fn at_150_percent_every_size_scales() {
        let mon = Monitor { work: r(0, 0, 2560, 1400), scale: 1.5 };
        let (first, all) = line(1000, 400, 1600, 430);
        let p = card_place(first, all, mon, CARD).unwrap();
        assert_eq!(p, CardPlace { rect: r(1084, 400, 1741, 931), below: true, arrow_x: 60.0, max_height: 300.0 });
    }

    #[test]
    fn short_room_makes_a_shorter_card() {
        let p = card_place(r(500, 200, 900, 220), r(500, 200, 900, 820), FHD, CARD).unwrap();
        assert_eq!((p.below, p.max_height, p.rect.top), (true, 194.0, 800));
    }

    #[test]
    fn no_room_for_a_useful_card_means_the_tray() {
        assert_eq!(card_place(r(100, 100, 1800, 120), r(100, 100, 1800, 950), FHD, CARD), None);
    }

    #[test]
    fn a_selection_off_the_work_area_means_the_tray() {
        let (first, all) = line(500, 1100, 900, 1120);
        assert_eq!(card_place(first, all, FHD, CARD), None);
        let narrow = Monitor { work: r(0, 0, 400, 1032), scale: 1.0 };
        let (first, all) = line(10, 300, 200, 320);
        assert_eq!(card_place(first, all, narrow, CARD), None, "narrower than the card");
    }

    #[test]
    fn a_monitor_left_of_the_primary_has_negative_coordinates() {
        let mon = Monitor { work: r(-1920, 0, 0, 1032), scale: 1.0 };
        let (first, all) = line(-1500, 300, -1100, 320);
        assert_eq!(card_place(first, all, mon, CARD).unwrap().rect, r(-1444, 300, -1006, 654));
    }
}
