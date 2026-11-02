//! Tray icon: the CUE dictation mark as the Drop shows it at rest, five voice bars with round ends and a lamp at their
//! top right (the mark's own proportions, still). The mark is drawn to its own frame: most of the icon height, centered.
//! Idle is monochrome; recording has a red, larger lamp; pause is faded.
use cue_core::ui::{UiPhase, UiState};

/// Share of the icon height the mark's frame may take, and of its width.
const MARK_HEIGHT: f32 = 0.90;
const MARK_WIDTH: f32 = 0.92;
// The mark on the Drop's 24 grid (cue.js, the idle pose): bars at x 4.5 … 18.5, 7 / 13 / 16 / 12 / 7 high, centered
// on y 12, a 2.2 stroke; the lamp at (21.6, 4.2), r 1.9, 2.4 while recording.
const GRID_BARS_X: [f32; 5] = [4.5, 8.0, 11.5, 15.0, 18.5];
const GRID_BARS_H: [f32; 5] = [7.0, 13.0, 16.0, 12.0, 7.0];
const GRID_BAR_CY: f32 = 12.0;
const GRID_HALF_STROKE: f32 = 1.1;
const GRID_LAMP: (f32, f32) = (21.6, 4.2);
const GRID_LAMP_IDLE: f32 = 1.9;
const GRID_LAMP_REC: f32 = 2.4;
// The frame on the grid, sized by the large lamp so the mark does not shift while recording: from the first bar's
// left edge to the large lamp's right edge, from the large lamp's top to the tallest bar's bottom.
const GRID_LEFT: f32 = 4.5 - GRID_HALF_STROKE;
const GRID_RIGHT: f32 = 21.6 + GRID_LAMP_REC;
const GRID_TOP: f32 = 4.2 - GRID_LAMP_REC;
const GRID_BOTTOM: f32 = GRID_BAR_CY + 8.0 + GRID_HALF_STROKE;

/// The mark in px of an icon of `size`.
#[derive(Clone, Copy, Debug)]
struct Mark {
    /// Horizontal centers of the bars, and the ends of their axes (without the round caps).
    bars_x: [f32; 5],
    bars_top: [f32; 5],
    bars_bottom: [f32; 5],
    half_stroke: f32,
    lamp_x: f32,
    lamp_y: f32,
    lamp_idle: f32,
    lamp_rec: f32,
}

impl Mark {
    /// The mark's frame is centered in the icon, its left edge on a pixel boundary: crisp bar edges even at 16 px.
    fn new(size: u32) -> Self {
        let s = size as f32;
        let (w, h) = (GRID_RIGHT - GRID_LEFT, GRID_BOTTOM - GRID_TOP);
        let unit = (MARK_HEIGHT * s / h).min(MARK_WIDTH * s / w);
        let left = ((s - w * unit) / 2.0).round();
        let top = (s - h * unit) / 2.0;
        let x = |g: f32| left + (g - GRID_LEFT) * unit;
        let y = |g: f32| top + (g - GRID_TOP) * unit;
        Self {
            bars_x: GRID_BARS_X.map(x),
            bars_top: GRID_BARS_H.map(|h| y(GRID_BAR_CY - h / 2.0)),
            bars_bottom: GRID_BARS_H.map(|h| y(GRID_BAR_CY + h / 2.0)),
            half_stroke: GRID_HALF_STROKE * unit,
            lamp_x: x(GRID_LAMP.0),
            lamp_y: y(GRID_LAMP.1),
            lamp_idle: GRID_LAMP_IDLE * unit,
            lamp_rec: GRID_LAMP_REC * unit,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayLook {
    Idle,
    Recording,
    Paused,
}

impl TrayLook {
    /// Icon look from the Drop state: pause wins; start-up and recording get the red lamp; the rest is idle.
    pub fn of(state: &UiState) -> Self {
        if state.paused {
            TrayLook::Paused
        } else if matches!(state.phase, UiPhase::Start | UiPhase::Listen) {
            TrayLook::Recording
        } else {
            TrayLook::Idle
        }
    }
}

/// Icon of `size`×`size`, RGBA row by row. Anti-aliased edges: pixel coverage comes from the distance between
/// its center and the shape.
pub fn rgba(look: TrayLook, light_taskbar: bool, size: u32) -> Vec<u8> {
    let m = Mark::new(size);
    let ink: [u8; 3] = if light_taskbar { [0x1C, 0x1C, 0x1F] } else { [0xF4, 0xF4, 0xF6] };
    let (lamp, lamp_r) = if look == TrayLook::Recording { ([0xFF, 0x4A, 0x1C], m.lamp_rec) } else { (ink, m.lamp_idle) };
    let fade = if look == TrayLook::Paused { 0.45 } else { 1.0 };
    // Distance to the shape edge in px (negative inside) → covered share of the pixel.
    let cover = |d: f32| (0.5 - d).clamp(0.0, 1.0);
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Each bar is a vertical line with round ends; the lamp is a circle.
            let bars = (0..5)
                .map(|i| {
                    let dy = (m.bars_top[i] - cy).max(cy - m.bars_bottom[i]).max(0.0);
                    cover((cx - m.bars_x[i]).hypot(dy) - m.half_stroke)
                })
                .fold(0.0f32, f32::max);
            let dot = cover((cx - m.lamp_x).hypot(cy - m.lamp_y) - lamp_r);
            let (rgb, a) = if dot >= bars { (lamp, dot) } else { (ink, bars) };
            let i = ((y * size + x) * 4) as usize;
            out[i..i + 3].copy_from_slice(&rgb);
            out[i + 3] = (a * fade * 255.0).round() as u8;
        }
    }
    out
}

/// Small icon size unknown: draw larger, the shell scales it down itself.
const FALLBACK_SIZE: u32 = 32;
/// Never draw larger than this: at a very large scale the shell scales it up itself.
const MAX_SIZE: u32 = 64;

/// Small icon size from `GetSystemMetricsForDpi` → image size: below 16 (including 0, unknown) gives
/// `FALLBACK_SIZE`, above `MAX_SIZE` gives `MAX_SIZE`.
fn icon_px(px: i32) -> u32 {
    match u32::try_from(px) {
        Ok(p) if p >= 16 => p.min(MAX_SIZE),
        _ => FALLBACK_SIZE,
    }
}

/// Icon size, px: the Windows small icon at the current scale of the primary monitor (where the tray icons are):
/// 16 at 100 %, 24 at 150 %, at most 64. We draw exactly to it, so the shell does not need to resample the image.
pub fn icon_size() -> u32 {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint};
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::WindowsAndMessaging::SM_CXSMICON;
    let (mut x, mut y) = (0u32, 0u32);
    // SAFETY: a point and constants; the monitor comes from MonitorFromPoint (with DEFAULTTOPRIMARY there always is
    // one); the pointers are to our own variables. GetSystemMetricsForDpi accepts any number.
    let px = unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        match GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) {
            Ok(()) if x > 0 => GetSystemMetricsForDpi(SM_CXSMICON, x),
            _ => 0,
        }
    };
    icon_px(px)
}

/// Taskbar theme: `HKCU\…\Themes\Personalize\SystemUsesLightTheme = 1` means light. If it cannot be read, dark
/// (the default look of the Windows 11 taskbar).
pub fn light_taskbar() -> bool {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows::core::w;
    let mut value: u32 = 0;
    let mut len = std::mem::size_of::<u32>() as u32;
    // SAFETY: the strings are `w!` constants; the data goes into our own `u32` variable, `len` is its size
    // (RRF_RT_REG_DWORD will not let more be written).
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut len),
        )
    };
    r.is_ok() && value == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use cue_core::ui::Mode;

    fn px(img: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [img[i], img[i + 1], img[i + 2], img[i + 3]]
    }

    const DARK_INK: [u8; 3] = [0xF4, 0xF4, 0xF6];
    const LIGHT_INK: [u8; 3] = [0x1C, 0x1C, 0x1F];
    const RED: [u8; 3] = [0xFF, 0x4A, 0x1C];
    /// 16 px is 100 % scale, 24 px is 150 %, 32 px is 200 % (and the fallback size).
    const SIZES: [u32; 3] = [16, 24, 32];

    /// The mark's frame by pixels covered at least halfway: (left, top, right, bottom), inclusive.
    fn solid_bbox(img: &[u8], size: u32) -> (u32, u32, u32, u32) {
        let solid: Vec<(u32, u32)> =
            (0..size).flat_map(|y| (0..size).map(move |x| (x, y))).filter(|&(x, y)| px(img, size, x, y)[3] >= 128).collect();
        let xs = solid.iter().map(|p| p.0);
        let ys = solid.iter().map(|p| p.1);
        (xs.clone().min().unwrap(), ys.clone().min().unwrap(), xs.max().unwrap(), ys.max().unwrap())
    }

    /// The pixel under the lamp's center, and the alpha in a box around it: the lamp's size.
    fn lamp(img: &[u8], size: u32) -> ([u8; 4], u32) {
        let m = Mark::new(size);
        let center = px(img, size, m.lamp_x as u32, m.lamp_y as u32);
        let r = m.lamp_rec + 1.0;
        let (x0, x1) = ((m.lamp_x - r).max(0.0) as u32, ((m.lamp_x + r) as u32).min(size - 1));
        let (y0, y1) = ((m.lamp_y - r).max(0.0) as u32, ((m.lamp_y + r) as u32).min(size - 1));
        let weight = (y0..=y1).flat_map(|y| (x0..=x1).map(move |x| (x, y))).map(|(x, y)| u32::from(px(img, size, x, y)[3])).sum();
        (center, weight)
    }

    /// Columns where the middle row's alpha peaks: one per voice bar.
    fn bar_peaks(img: &[u8], size: u32) -> usize {
        let y = size / 2;
        let a: Vec<u8> = (0..size).map(|x| px(img, size, x, y)[3]).collect();
        (0..a.len())
            .filter(|&i| a[i] >= 160 && (i == 0 || a[i] > a[i - 1]) && (i + 1 == a.len() || a[i] >= a[i + 1]))
            .count()
    }

    #[test]
    fn mark_is_drawn_at_every_real_size() {
        for size in SIZES {
            for (light, ink) in [(false, DARK_INK), (true, LIGHT_INK)] {
                let idle = rgba(TrayLook::Idle, light, size);
                assert_eq!(idle.len(), (size * size * 4) as usize, "{size} px");
                let m = Mark::new(size);
                let mid = px(&idle, size, m.bars_x[2] as u32, size / 2);
                assert_eq!(mid, [ink[0], ink[1], ink[2], 255], "{size} px: the middle bar is opaque, taskbar color");
                let (lamp_px, _) = lamp(&idle, size);
                assert_eq!(lamp_px, [ink[0], ink[1], ink[2], 255], "{size} px: idle lamp in the bars' color");
                let rec = rgba(TrayLook::Recording, light, size);
                assert_eq!(lamp(&rec, size).0, [RED[0], RED[1], RED[2], 255], "{size} px: recording lamp is red");
                let paused = rgba(TrayLook::Paused, light, size);
                assert!(paused.chunks(4).all(|p| p[3] <= 115), "{size} px: pause is faded");
                assert_eq!(px(&paused, size, m.bars_x[2] as u32, size / 2)[3], 115, "{size} px");
                for img in [&idle, &rec, &paused] {
                    for (x, y) in [(0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1)] {
                        assert_eq!(px(img, size, x, y)[3], 0, "{size} px: corner ({x}, {y}) is transparent");
                    }
                }
            }
        }
    }

    #[test]
    fn mark_is_five_voice_bars() {
        for size in [24, 32, 48, 64] {
            let img = rgba(TrayLook::Idle, false, size);
            assert_eq!(bar_peaks(&img, size), 5, "{size} px: five bars across the middle row");
        }
    }

    #[test]
    fn mark_fills_most_of_the_height_and_is_centered() {
        for size in 16..=64 {
            for look in [TrayLook::Idle, TrayLook::Recording] {
                let img = rgba(look, false, size);
                let (left, top, right, bottom) = solid_bbox(&img, size);
                let height = (bottom - top + 1) as f32 / size as f32;
                assert!((0.78..=0.95).contains(&height), "{size} px {look:?}: the mark is {height:.2} of the height");
                // The frame is sized by the large lamp: at rest the smaller lamp leaves a little more air above.
                let (above, below) = (top, size - 1 - bottom);
                let slack = if look == TrayLook::Idle { 1 + size / 32 } else { 1 };
                assert!(above.abs_diff(below) <= slack, "{size} px {look:?}: not centered vertically ({above} / {below})");
                if look == TrayLook::Recording {
                    // The mark's frame is sized by the large lamp: the mark does not shift while recording.
                    let (before, after) = (left, size - 1 - right);
                    assert!(before.abs_diff(after) <= 1, "{size} px: not centered horizontally ({before} / {after})");
                }
            }
        }
    }

    #[test]
    fn bars_are_thick_enough() {
        for size in SIZES {
            let img = rgba(TrayLook::Idle, false, size);
            let m = Mark::new(size);
            let y = size / 2;
            let x = m.bars_x[2];
            let width: f32 = ((x - 2.0) as u32..=(x + 2.0) as u32).map(|x| f32::from(px(&img, size, x, y)[3]) / 255.0).sum();
            assert!(width >= 1.4 * size as f32 / 16.0, "{size} px: bar thickness {width:.2} px");
        }
    }

    #[test]
    fn recording_lamp_is_larger() {
        for size in SIZES {
            let idle = lamp(&rgba(TrayLook::Idle, false, size), size).1;
            let rec = lamp(&rgba(TrayLook::Recording, false, size), size).1;
            assert!(rec * 10 > idle * 13, "{size} px: the recording lamp is clearly larger ({rec} vs {idle})");
        }
    }

    /// Writes the icons as raw RGBA (`CUE_TRAY_DUMP` is the folder) to look at them: never asserted.
    #[test]
    #[ignore = "writes files to look at (CUE_TRAY_DUMP)"]
    fn dump_icons() {
        let dir = std::path::PathBuf::from(std::env::var("CUE_TRAY_DUMP").expect("CUE_TRAY_DUMP"));
        for size in [16, 24, 32, 64] {
            for (look, name) in [(TrayLook::Idle, "idle"), (TrayLook::Recording, "rec"), (TrayLook::Paused, "paused")] {
                for (light, theme) in [(false, "dark"), (true, "light")] {
                    std::fs::write(dir.join(format!("{name}-{theme}-{size}.rgba")), rgba(look, light, size)).unwrap();
                }
            }
        }
    }

    #[test]
    fn icon_size_is_clamped() {
        assert_eq!(icon_px(16), 16);
        assert_eq!(icon_px(24), 24);
        assert_eq!(icon_px(64), 64);
        assert_eq!(icon_px(80), 64, "very large scale: at most 64");
        assert_eq!(icon_px(0), 32, "unknown");
        assert_eq!(icon_px(-1), 32);
        assert_eq!(icon_px(12), 32);
    }

    #[test]
    fn look_follows_core_state() {
        let ui = |phase, paused| UiState { phase, error: None, retry: false, paused, mode: Mode::Dictate };
        assert_eq!(TrayLook::of(&ui(UiPhase::Start, false)), TrayLook::Recording);
        assert_eq!(TrayLook::of(&ui(UiPhase::Listen, false)), TrayLook::Recording);
        for phase in [UiPhase::Idle, UiPhase::Think, UiPhase::Done, UiPhase::Empty, UiPhase::Cancel, UiPhase::Error] {
            assert_eq!(TrayLook::of(&ui(phase, false)), TrayLook::Idle, "{phase:?}");
        }
        // Pause during "Thinking" (the dictation is still being finished): the icon is already faded.
        for phase in [UiPhase::Idle, UiPhase::Listen, UiPhase::Think] {
            assert_eq!(TrayLook::of(&ui(phase, true)), TrayLook::Paused, "{phase:?}");
        }
    }
}
