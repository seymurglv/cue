//! Tray icon and its interim menu (until the mini tray replaces it).
//!
//! The core drives the icon look and the "Pause for an hour" check (`sync` from `Runtime::emit_state`). A dedicated
//! thread re-reads the taskbar theme and icon size every 5 s; it also asks the main thread every 5 s to reconcile
//! the autostart check with the Run key. A change of the interface language relabels the menu (`relabel`). No dictation
//! text and no audio here.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use cue_core::flow::Input;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, MenuItemKind, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_autostart::ManagerExt;
use tokio::sync::mpsc::UnboundedSender;

use crate::lang::Lang;
use crate::runtime::CoreEvent;
use crate::trayicon::{TrayLook, icon_size, light_taskbar, rgba};

pub const TRAY_ID: &str = "cue";
/// How often the taskbar theme, the icon size and the autostart key are re-read.
const WATCH_EVERY: Duration = Duration::from_secs(5);

/// Menu item ids, as `on_menu` tells the items apart.
const PAUSE: &str = "pause";
const AUTOSTART: &str = "autostart";
const LOG: &str = "log";
const QUIT: &str = "quit";

/// Menu item labels in one language.
struct MenuText {
    pause: &'static str,
    autostart: &'static str,
    log: &'static str,
    quit: &'static str,
}

fn menu_text(lang: Lang) -> MenuText {
    match lang {
        Lang::En => MenuText { pause: "Pause for an hour", autostart: "Start at sign-in", log: "Open log", quit: "Quit" },
        Lang::Ru => MenuText { pause: "Пауза на час", autostart: "Запускать при входе", log: "Открыть журнал", quit: "Выход" },
    }
}

/// Every item of the menu with its label in `lang`, in menu order: the menu is built from it, and `relabel` goes over
/// it, so no item keeps the old language.
fn labels(lang: Lang) -> [(&'static str, &'static str); 4] {
    let t = menu_text(lang);
    [(PAUSE, t.pause), (AUTOSTART, t.autostart), (LOG, t.log), (QUIT, t.quit)]
}

/// What the icon should show. The redraw (on the main thread) reads everything from here when it runs:
/// whichever redraw runs last shows the latest state.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Shown {
    look: TrayLook,
    paused: bool,
    light: bool,
    size: u32,
}

struct Tray {
    icon: TrayIcon<Wry>,
    /// The whole menu, for `relabel`.
    menu: Menu<Wry>,
    pause: CheckMenuItem<Wry>,
    autostart: CheckMenuItem<Wry>,
    shown: Mutex<Shown>,
    /// What the autostart check showed last. Written and compared only on the main thread.
    autostart_on: AtomicBool,
}

impl Tray {
    fn shown(&self) -> std::sync::MutexGuard<'_, Shown> {
        self.shown.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn image(s: &Shown) -> Image<'static> {
    Image::new_owned(rgba(s.look, s.light, s.size), s.size, s.size)
}

/// Icon and menu, the menu in the interface language. Called once from `setup`, before the core starts.
pub fn create(app: &AppHandle, core: UnboundedSender<CoreEvent>, logs: PathBuf, lang: Lang) -> tauri::Result<()> {
    let shown = Shown { look: TrayLook::Idle, paused: false, light: light_taskbar(), size: icon_size() };
    // Autostart is the plugin's Run key, not a setting: the check shows the key as it is.
    let autostart_on = app.autolaunch().is_enabled().unwrap_or_else(|e| {
        tracing::warn!("autostart: state not read: {e}");
        false
    });
    let [(pause_id, pause), (autostart_id, autostart), (log_id, log), (quit_id, quit)] = labels(lang);
    let pause = CheckMenuItem::with_id(app, pause_id, pause, true, false, None::<&str>)?;
    let autostart = CheckMenuItem::with_id(app, autostart_id, autostart, true, autostart_on, None::<&str>)?;
    let log = MenuItem::with_id(app, log_id, log, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, quit_id, quit, true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&pause, &autostart, &sep, &log, &quit])?;
    let icon = TrayIconBuilder::with_id(TRAY_ID)
        .icon(image(&shown))
        .tooltip("CUE · Ctrl + Win")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, ev| on_menu(app, &ev, &core, &logs))
        .build(app)?;
    app.manage(Tray {
        icon,
        menu,
        pause,
        autostart,
        shown: Mutex::new(shown),
        autostart_on: AtomicBool::new(autostart_on),
    });
    tracing::info!(light = shown.light, size = shown.size, autostart = autostart_on, "tray icon created");
    spawn_watch(app.clone());
    Ok(())
}

/// Icon look and the "Pause for an hour" check follow the core state (also when the hour of pause runs out).
/// Redraws only on a change; does not wait for the main thread.
pub fn sync(app: &AppHandle, look: TrayLook, paused: bool) {
    let Some(tray) = app.try_state::<Tray>() else { return };
    {
        let mut s = tray.shown();
        if (s.look, s.paused) == (look, paused) {
            return;
        }
        s.look = look;
        s.paused = paused;
    }
    redraw(app);
}

fn redraw(app: &AppHandle) {
    let handle = app.clone();
    let posted = app.run_on_main_thread(move || {
        let Some(tray) = handle.try_state::<Tray>() else { return };
        let s = *tray.shown();
        if let Err(e) = tray.icon.set_icon(Some(image(&s))) {
            tracing::warn!("tray icon not redrawn: {e}");
        }
        if let Err(e) = tray.pause.set_checked(s.paused) {
            tracing::warn!("pause check not updated: {e}");
        }
    });
    if let Err(e) = posted {
        // The app is closing.
        tracing::debug!("tray icon: redraw not posted: {e}");
    }
}

/// The menu in another interface language (decision 5): every item takes its label in `lang`. Main thread.
pub fn relabel(app: &AppHandle, lang: Lang) {
    let Some(tray) = app.try_state::<Tray>() else { return };
    for (id, label) in labels(lang) {
        let relabeled = match tray.menu.get(id) {
            Some(MenuItemKind::MenuItem(item)) => item.set_text(label),
            Some(MenuItemKind::Check(item)) => item.set_text(label),
            _ => {
                tracing::warn!("tray menu: no item {id} to relabel");
                continue;
            }
        };
        if let Err(e) = relabeled {
            tracing::warn!("tray menu: item {id} not relabeled: {e}");
        }
    }
}

/// Menu items (main thread).
fn on_menu(app: &AppHandle, ev: &MenuEvent, core: &UnboundedSender<CoreEvent>, logs: &Path) {
    let Some(tray) = app.try_state::<Tray>() else { return };
    match ev.id().as_ref() {
        PAUSE => {
            // The menu has already flipped the check, but the core decides: keep the old state until its answer arrives in `sync`.
            let paused = tray.shown().paused;
            let _ = tray.pause.set_checked(paused);
            if core.send(CoreEvent::Input(Input::TogglePause)).is_err() {
                tracing::warn!("pause: the core is not responding");
            }
        }
        AUTOSTART => toggle_autostart(app, &tray),
        LOG => open_logs(logs),
        QUIT => {
            tracing::info!("quit from the tray menu");
            app.exit(0);
        }
        _ => {}
    }
}

/// Autostart is the plugin's Run key. An error is one log line; afterwards the check follows the key as it is.
fn toggle_autostart(app: &AppHandle, tray: &Tray) {
    let m = app.autolaunch();
    let toggled = m.is_enabled().and_then(|on| if on { m.disable() } else { m.enable() });
    let on = m.is_enabled().unwrap_or(false);
    match toggled {
        Ok(()) => tracing::info!(on, "start at sign-in toggled"),
        Err(e) => tracing::warn!(on, "start at sign-in not toggled: {e}"),
    }
    show_autostart(tray, on);
}

/// The autostart check follows the Run key when it was changed outside the menu (Task Manager, Windows Settings).
/// Main thread only, like `toggle_autostart`: no click can slip in between reading the key and setting the check.
/// If the key cannot be read, the check is left alone (the error is logged when toggling from the menu).
fn refresh_autostart(app: &AppHandle) {
    let Some(tray) = app.try_state::<Tray>() else { return };
    if let Ok(on) = app.autolaunch().is_enabled()
        && on != tray.autostart_on.load(Ordering::Relaxed)
    {
        show_autostart(&tray, on);
    }
}

/// Main thread (menu and `refresh_autostart`).
fn show_autostart(tray: &Tray, on: bool) {
    tray.autostart_on.store(on, Ordering::Relaxed);
    if let Err(e) = tray.autostart.set_checked(on) {
        tracing::warn!("autostart check not updated: {e}");
    }
}

/// The log folder in File Explorer (created if missing).
fn open_logs(logs: &Path) {
    if let Err(e) = std::fs::create_dir_all(logs) {
        tracing::warn!("log folder not created: {e}");
    }
    if let Err(e) = std::process::Command::new("explorer.exe").arg(logs).spawn() {
        tracing::warn!("log folder did not open: {e}");
    }
}

/// Every 5 s: if the taskbar theme or the icon size (primary monitor scale) changed, redraw; if the autostart key
/// was changed outside the menu (Task Manager, Windows Settings), fix the check.
fn spawn_watch(app: AppHandle) {
    let spawned = std::thread::Builder::new().name("cue-tray-watch".into()).spawn(move || {
        loop {
            std::thread::sleep(WATCH_EVERY);
            let Some(tray) = app.try_state::<Tray>() else { return };
            let (light, size) = (light_taskbar(), icon_size());
            let changed = {
                let mut s = tray.shown();
                let changed = (s.light, s.size) != (light, size);
                (s.light, s.size) = (light, size);
                changed
            };
            if changed {
                tracing::info!(light, size, "tray icon: taskbar theme or scale changed");
                redraw(&app);
            }
            // Reconcile on the main thread, in turn with toggles from the menu: a state read here before a click
            // cannot overwrite the one just toggled.
            let handle = app.clone();
            if let Err(e) = app.run_on_main_thread(move || refresh_autostart(&handle)) {
                tracing::debug!("tray icon: autostart reconcile not posted: {e}");
            }
        }
    });
    if let Err(e) = spawned {
        tracing::warn!("tray icon thread not started: the taskbar theme is read only at startup ({e})");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both languages name every item of the menu, in the same order, each in its own words: a change of the interface
    /// language leaves no item in the old one.
    #[test]
    fn tray_menu_relabels_in_both_languages() {
        let (en, ru) = (labels(Lang::En), labels(Lang::Ru));
        let ids = |items: &[(&'static str, &'static str)]| items.iter().map(|(id, _)| *id).collect::<Vec<_>>();
        assert_eq!(ids(&en), [PAUSE, AUTOSTART, LOG, QUIT]);
        assert_eq!(ids(&ru), ids(&en));
        for ((id, en), (_, ru)) in en.iter().zip(&ru) {
            assert!(!en.is_empty() && !ru.is_empty() && en != ru, "{id}: {en:?} / {ru:?}");
        }
        assert_eq!(en.map(|(_, label)| label), ["Pause for an hour", "Start at sign-in", "Open log", "Quit"]);
        assert_eq!(ru.map(|(_, label)| label), ["Пауза на час", "Запускать при входе", "Открыть журнал", "Выход"]);
    }
}
