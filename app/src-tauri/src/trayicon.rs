//! Tray icon: the caret of the CUE mark, a vertical line with round ends and a lamp at the top right (the mark's own
//! proportions). The mark is drawn to its own frame: 90 % of the icon height, centered. Idle is monochrome; recording
//! has a red lamp; pause is faded.
use cue_core::ui::{UiPhase, UiState};

/// Share of the icon height the mark takes (from the top to the bottom of the line).
const MARK_HEIGHT: f32 = 0.90;
/// Line thickness as a share of the icon size: 2.24 px at 16 px.
const LINE_WIDTH: f32 = 0.14;
// Mark proportions (24×24 grid): line from (11, 5) to (11, 19), 2.2 thick, so the mark is 16.2 high; the lamp center
// is 6.4 right of the line; lamp radius 1.9, 2.4 while recording. The lamp grows with the line thickness.
const GRID_HEIGHT: f32 = 16.2;
const GRID_HALF_LINE: f32 = 1.1;
const GRID_LAMP_DX: f32 = 6.4;
const GRID_LAMP_IDLE: f32 = 1.9;
const GRID_LAMP_REC: f32 = 2.4;

/// The mark in px of an icon of `size`.
#[derive(Clone, Copy, Debug)]
struct Mark {
    /// Horizontal center of the line and the ends of its axis (without the round caps).
    line_x: f32,
    line_top: f32,
    line_bottom: f32,
    half_line: f32,
    lamp_x: f32,
    lamp_y: f32,
    lamp_idle: f32,
    lamp_rec: f32,
}

impl Mark {
    /// The mark's frame (sized by the large lamp, so the mark does not shift while recording) is centered in the icon;
    /// the left edge of the line is on a pixel boundary: a crisp edge even at 16 px.
    fn new(size: u32) -> Self {
        let s = size as f32;
        let unit = MARK_HEIGHT * s / GRID_HEIGHT;
        let half_line = LINE_WIDTH * s / 2.0;
        let grow = half_line / GRID_HALF_LINE;
        let (lamp_idle, lamp_rec) = (GRID_LAMP_IDLE * grow, GRID_LAMP_REC * grow);
        let dx = GRID_LAMP_DX * unit;
        let top = (1.0 - MARK_HEIGHT) / 2.0 * s;
        // From the left edge of the line to the right edge of the large lamp.
        let width = half_line + dx + lamp_rec;
        let line_x = ((s - width) / 2.0).round() + half_line;
        Self {
            line_x,
            line_top: top + half_line,
            line_bottom: s - top - half_line,
            half_line,
            lamp_x: line_x + dx,
            // The top of the large lamp is level with the top of the line.
            lamp_y: top + lamp_rec,
            lamp_idle,
            lamp_rec,
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
            let dy = if cy < m.line_top {
                m.line_top - cy
            } else if cy > m.line_bottom {
                cy - m.line_bottom
            } else {
                0.0
            };
            // A line with round ends; the lamp is a circle.
            let line = cover((cx - m.line_x).hypot(dy) - m.half_line);
            let dot = cover((cx - m.lamp_x).hypot(cy - m.lamp_y) - lamp_r);
            let (rgb, a) = if dot >= line { (lamp, dot) } else { (ink, line) };
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

    /// Line columns in the middle row (only the line is there): the first and last pixel with alpha > 0.
    fn line_span(img: &[u8], size: u32) -> (u32, u32) {
        let y = size / 2;
        let first = (0..size).find(|&x| px(img, size, x, y)[3] > 0).expect("line in the middle row");
        let last = (first..size).take_while(|&x| px(img, size, x, y)[3] > 0).last().unwrap();
        (first, last)
    }

    /// The mark's frame by pixels covered at least halfway: (left, top, right, bottom), inclusive.
    fn solid_bbox(img: &[u8], size: u32) -> (u32, u32, u32, u32) {
        let solid: Vec<(u32, u32)> =
            (0..size).flat_map(|y| (0..size).map(move |x| (x, y))).filter(|&(x, y)| px(img, size, x, y)[3] >= 128).collect();
        let xs = solid.iter().map(|p| p.0);
        let ys = solid.iter().map(|p| p.1);
        (xs.clone().min().unwrap(), ys.clone().min().unwrap(), xs.max().unwrap(), ys.max().unwrap())
    }

    /// The lamp pixel: the center of mass of the pixels right of the line (weighted by alpha).
    fn lamp_center(img: &[u8], size: u32) -> (u32, u32) {
        let (_, line_end) = line_span(img, size);
        let (mut sx, mut sy, mut sa) = (0.0, 0.0, 0.0);
        for y in 0..size {
            for x in line_end + 1..size {
                let a = f32::from(px(img, size, x, y)[3]);
                (sx, sy, sa) = (sx + a * (x as f32 + 0.5), sy + a * (y as f32 + 0.5), sa + a);
            }
        }
        assert!(sa > 0.0, "{size} px: there is a lamp");
        ((sx / sa) as u32, (sy / sa) as u32)
    }

    /// How much alpha is right of the line: the lamp size.
    fn lamp_weight(img: &[u8], size: u32) -> u32 {
        let (_, line_end) = line_span(img, size);
        (0..size).flat_map(|y| (line_end + 1..size).map(move |x| (x, y))).map(|(x, y)| u32::from(px(img, size, x, y)[3])).sum()
    }

    #[test]
    fn mark_is_drawn_at_every_real_size() {
        for size in SIZES {
            for (light, ink) in [(false, DARK_INK), (true, LIGHT_INK)] {
                let idle = rgba(TrayLook::Idle, light, size);
                assert_eq!(idle.len(), (size * size * 4) as usize, "{size} px");
                let (first, last) = line_span(&idle, size);
                let mid = px(&idle, size, (first + last) / 2, size / 2);
                assert_eq!(mid, [ink[0], ink[1], ink[2], 255], "{size} px: mid-line is opaque, taskbar color");
                let (lx, ly) = lamp_center(&idle, size);
                assert_eq!(px(&idle, size, lx, ly), [ink[0], ink[1], ink[2], 255], "{size} px: idle lamp in line color");
                let rec = rgba(TrayLook::Recording, light, size);
                let (rx, ry) = lamp_center(&rec, size);
                assert_eq!(px(&rec, size, rx, ry), [RED[0], RED[1], RED[2], 255], "{size} px: recording lamp is red");
                let paused = rgba(TrayLook::Paused, light, size);
                assert!(paused.chunks(4).all(|p| p[3] <= 115), "{size} px: pause is faded");
                assert_eq!(px(&paused, size, (first + last) / 2, size / 2)[3], 115, "{size} px");
                for img in [&idle, &rec, &paused] {
                    for (x, y) in [(0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1)] {
                        assert_eq!(px(img, size, x, y)[3], 0, "{size} px: corner ({x}, {y}) is transparent");
                    }
                }
            }
        }
    }

    #[test]
    fn mark_fills_ninety_percent_of_height_and_is_centered() {
        for size in 16..=64 {
            for look in [TrayLook::Idle, TrayLook::Recording] {
                let img = rgba(look, false, size);
                let (left, top, right, bottom) = solid_bbox(&img, size);
                let height = (bottom - top + 1) as f32 / size as f32;
                assert!((0.85..=0.95).contains(&height), "{size} px {look:?}: the mark is {height:.2} of the height");
                let (above, below) = (top, size - 1 - bottom);
                assert!(above.abs_diff(below) <= 1, "{size} px {look:?}: not centered vertically ({above} / {below})");
                if look == TrayLook::Recording {
                    // The mark's frame is sized by the large lamp: the mark does not shift while recording.
                    let (before, after) = (left, size - 1 - right);
                    assert!(before.abs_diff(after) <= 1, "{size} px: not centered horizontally ({before} / {after})");
                }
            }
        }
    }

    #[test]
    fn line_is_thick_enough() {
        for size in SIZES {
            let img = rgba(TrayLook::Idle, false, size);
            let y = size / 2;
            let opaque = (0..size).filter(|&x| px(&img, size, x, y)[3] == 255).count() as u32;
            assert!(opaque >= size / 8, "{size} px: the line has {opaque} opaque pixels");
            let width: f32 = (0..size).map(|x| f32::from(px(&img, size, x, y)[3]) / 255.0).sum();
            assert!(width >= 2.0 * size as f32 / 16.0, "{size} px: line thickness {width:.2} px");
        }
    }

    #[test]
    fn recording_lamp_is_larger() {
        for size in SIZES {
            let idle = lamp_weight(&rgba(TrayLook::Idle, false, size), size);
            let rec = lamp_weight(&rgba(TrayLook::Recording, false, size), size);
            assert!(rec * 10 > idle * 13, "{size} px: the recording lamp is clearly larger ({rec} vs {idle})");
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
