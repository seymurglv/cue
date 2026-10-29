//! What "Ask" does with the selection the focused field reported (spec 2 §6.1, §7): whether to copy it with Ctrl+C, and
//! where it is on screen. Pure logic.
use crate::focus::{Editability, SelectionRead};
use crate::placement::PxRect;

/// The Ctrl+C fallback is on. The live probe (spec 2 §13.5) switches it off if an app misbehaves with it.
pub const COPY_FALLBACK: bool = true;

/// Editors that copy their current line on Ctrl+C with nothing selected (spec 2 §13.5): the fallback would take that line
/// for a selection, so for it they count as terminals. Filled by the controller's probe after this task.
const COPIES_LINE: &[&str] = &[];

/// The app copies its current line on Ctrl+C when nothing is selected (`COPIES_LINE`).
pub fn copies_line(app: &str) -> bool {
    COPIES_LINE.iter().any(|a| app.eq_ignore_ascii_case(a))
}

/// Ctrl+C fallback (spec 2 §6.1): only when the field could not say whether anything is selected (no text pattern, not a
/// classic field), there may be a field at all (never in File Explorer's list or on the desktop, where Ctrl+C copies files),
/// the app is not a terminal (Ctrl+C there stops the running program; `terminal::terminal_app`), and Windows does not
/// upload what is copied to the cloud clipboard by itself (`clipboard::uploads_to_cloud`): the app's copy carries none of
/// CUE's marks, so the selection would reach the user's other devices (decision of 29.10). The selection then reads as
/// none.
pub fn needs_copy_fallback(read: &SelectionRead, editable: Editability, terminal: bool, cloud_upload: bool) -> bool {
    COPY_FALLBACK && matches!(read, SelectionRead::Unknown) && editable != Editability::No && !terminal && !cloud_upload
}

/// The selection on screen: its first line (the card's arrow points at its start) and the box around all of it. `None`
/// without a rectangle that has a size (spec 2 §7: then the tray).
pub fn bounds(rects: &[PxRect]) -> Option<(PxRect, PxRect)> {
    let mut shown = rects.iter().copied().filter(|r| r.width() > 0 && r.height() > 0);
    let first = shown.next()?;
    let all = shown.fold(first, |a, r| PxRect {
        left: a.left.min(r.left),
        top: a.top.min(r.top),
        right: a.right.max(r.right),
        bottom: a.bottom.max(r.bottom),
    });
    Some((first, all))
}

/// A rectangle UI Automation gives as left, top, width, height in physical px (doubles), rounded outwards; `None` for one
/// with no size or not a number.
pub fn px_rect(left: f64, top: f64, width: f64, height: f64) -> Option<PxRect> {
    let ok = [left, top, width, height].iter().all(|v| v.is_finite()) && width > 0.0 && height > 0.0;
    ok.then(|| PxRect {
        left: left.floor() as i32,
        top: top.floor() as i32,
        right: (left + width).ceil() as i32,
        bottom: (top + height).ceil() as i32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(left: i32, top: i32, right: i32, bottom: i32) -> PxRect {
        PxRect { left, top, right, bottom }
    }

    #[test]
    fn ctrl_c_only_when_the_field_cannot_say_and_never_in_a_terminal() {
        use Editability::*;
        let text = SelectionRead::Text { text: "x".into(), rects: vec![], read_only: false, clipped: false };
        for (read, editable, terminal, want) in [
            (&SelectionRead::Unknown, Yes, false, true),
            (&SelectionRead::Unknown, Unknown, false, true),
            (&SelectionRead::Unknown, No, false, false),
            (&SelectionRead::Unknown, Yes, true, false),
            (&SelectionRead::Empty, Yes, false, false),
            (&SelectionRead::Password, Yes, false, false),
            (&SelectionRead::NotAsked, Yes, false, false),
            (&text, Yes, false, false),
        ] {
            let got = needs_copy_fallback(read, editable, terminal, false);
            assert_eq!(got, want && COPY_FALLBACK, "{read:?} {editable:?} terminal={terminal}");
        }
    }

    /// Windows uploads what is copied to the cloud clipboard by itself: the app's copy would reach the user's other
    /// devices, so there is no Ctrl+C, and the selection reads as none.
    #[test]
    fn no_ctrl_c_while_windows_uploads_copies_to_the_cloud() {
        assert!(!needs_copy_fallback(&SelectionRead::Unknown, Editability::Yes, false, true));
        assert!(!needs_copy_fallback(&SelectionRead::Unknown, Editability::Unknown, false, true));
    }

    #[test]
    fn the_selection_on_screen() {
        assert_eq!(bounds(&[]), None);
        assert_eq!(bounds(&[r(10, 10, 10, 30)]), None, "no width: not on screen");
        let lines = [r(0, 0, 0, 0), r(300, 100, 900, 120), r(80, 120, 500, 140)];
        assert_eq!(bounds(&lines), Some((r(300, 100, 900, 120), r(80, 100, 900, 140))));
    }

    #[test]
    fn rectangles_from_ui_automation_round_outwards() {
        assert_eq!(px_rect(10.4, 20.6, 100.2, 18.0), Some(r(10, 20, 111, 39)));
        assert_eq!(px_rect(-1_920.5, 0.0, 10.0, 10.0), Some(r(-1_921, 0, -1_910, 10)), "a monitor left of the primary");
        for bad in [(0.0, 0.0, 0.0, 10.0), (0.0, 0.0, 10.0, -1.0), (f64::NAN, 0.0, 1.0, 1.0), (0.0, f64::INFINITY, 1.0, 1.0)] {
            assert_eq!(px_rect(bad.0, bad.1, bad.2, bad.3), None, "{bad:?}");
        }
    }
}
