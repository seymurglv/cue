#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod drop_window;
mod ipc;
mod lang;
mod logging;
mod paths;
mod runtime;
mod tray;
mod trayicon;

use std::sync::mpsc::Sender;

use anyhow::Context;

use cue_platform_win::chord::HotkeyEvent;
use cue_platform_win::hook::{self, HookError};
use cue_platform_win::power::{self, PowerEvent};
use tauri::Manager;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use runtime::CoreEvent;

fn main() {
    // Before anything else: no panic may print its message (it may contain dictation text).
    logging::install_panic_hook();
    let paths = match paths::Paths::from_env() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("CUE failed to start: {e}");
            std::process::exit(1);
        }
    };
    // The log goes first: later failures (plugins, window) must land in it too.
    if let Err(e) = logging::init(&paths.logs) {
        eprintln!("CUE: log not opened: {e}");
    }
    let (core_tx, core_rx) = tokio::sync::mpsc::unbounded_channel();
    let setup_tx = core_tx.clone();
    let built = tauri::Builder::default()
        // A second launch opens nothing.
        .plugin(tauri_plugin_single_instance::init(|_app, _argv, _cwd| {}))
        // The parameter matters only on macOS; on Windows autostart is the Run key.
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .manage(drop_window::HitState::default())
        .manage(ipc::CoreTx(core_tx))
        .invoke_handler(tauri::generate_handler![
            drop_window::drop_hit_rects,
            drop_window::drop_drag,
            ipc::drop_action,
            ipc::drop_ready
        ])
        .setup(move |app| {
            // We log the reason ourselves: Tauri turns a `setup` error into a panic, and the panic hook does not
            // print its message. The app then closes: without the tray icon CUE must not keep running invisibly.
            setup(app, paths, setup_tx, core_rx).inspect_err(|e| tracing::error!("CUE failed to start: {e:#}"))?;
            Ok(())
        })
        .build(tauri::generate_context!());
    match built {
        Ok(app) => app.run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                tracing::info!("CUE is closing");
                logging::flush();
            }
        }),
        Err(e) => {
            tracing::error!("CUE failed to start: {e}");
            logging::flush();
            std::process::exit(1);
        }
    }
}

fn setup(
    app: &mut tauri::App,
    paths: paths::Paths,
    core_tx: UnboundedSender<CoreEvent>,
    core_rx: UnboundedReceiver<CoreEvent>,
) -> anyhow::Result<()> {
    // Any read error (corrupt or unreadable) means defaults, and the user's file is not overwritten all session.
    let (settings, can_save_settings) = match cue_core::settings::load(&paths.settings) {
        Ok(s) => (s, true),
        Err(e) => {
            tracing::error!("settings: {e} — running with default settings and not saving them");
            (Default::default(), false)
        }
    };
    // Interface language: decided once here (settings.json first, then Windows);
    // the Drop window and the tray menu both get it.
    let lang = lang::Lang::resolve(settings.ui.language, lang::Lang::detect);
    tracing::info!(lang = lang.code(), setting = ?settings.ui.language, "interface language");
    // Counts only: the terms themselves stay out of the log.
    let terms = cue_core::refine::usable_terms(&settings.dictionary.terms).len();
    tracing::info!(terms, skipped = settings.dictionary.terms.len() - terms, "dictionary");
    // Where the Drop sits: settings.json, then changed only by dragging the Drop (saved by the core).
    let place = settings.drop.position;
    tracing::info!(place = place.code(), "Drop place");
    let ui = ipc::Ui::new(app.handle().clone(), place);
    let win = drop_window::create(app.handle(), lang, place)?;
    drop_window::spawn_hit_test(win.clone(), app.state::<drop_window::HitState>().inner().clone());
    let (drag_tx, drag_rx) = std::sync::mpsc::channel();
    app.manage(drop_window::Placer::new(drag_tx));
    let tx = core_tx.clone();
    drop_window::spawn_placement(win, ui.clone(), place, drag_rx, move |p| {
        let _ = tx.send(CoreEvent::DropMoved(p));
    });
    // Without the icon there is no way to quit or pause: if it is not created, CUE does not start (reason in the log).
    tray::create(app.handle(), core_tx.clone(), paths.logs.clone(), lang).context("tray icon not created")?;

    let (hk_tx, hk_rx) = std::sync::mpsc::channel::<HotkeyEvent>();
    start_hook(&hk_tx);
    let tx = core_tx.clone();
    std::thread::Builder::new().name("cue-hotkeys".into()).spawn(move || {
        for ev in hk_rx {
            if tx.send(CoreEvent::Input(runtime::input_of(ev))).is_err() {
                break;
            }
        }
    })?;
    // The handler is quick (Windows waits for it on suspend, ~2 s): reinstall the hook and message the core, that's all.
    // We do not rely on the event order around sleep: Locked and Suspending arrive together, Resumed before Unlocked.
    let tx = core_tx.clone();
    power::spawn(move |ev| {
        tracing::info!(?ev, "power");
        match ev {
            PowerEvent::Resumed | PowerEvent::Unlocked => {
                start_hook(&hk_tx);
                // Pooled connections are most likely dead after sleep: the next recording warms them up at once.
                let _ = tx.send(CoreEvent::Wake);
            }
            PowerEvent::Locked | PowerEvent::Suspending => {
                let _ = tx.send(CoreEvent::Input(cue_core::flow::Input::SessionLocked));
            }
        }
    });

    let rt = runtime::Runtime::new(ui, settings, can_save_settings, paths, core_tx);
    // The core blocks its thread on short Windows calls (UI Automation, microphone), so it gets its own thread;
    // tasks (network, timers) run on the shared tokio runtime.
    std::thread::Builder::new()
        .name("cue-core".into())
        .spawn(move || tauri::async_runtime::block_on(rt.run(core_rx)))?;
    tracing::info!("CUE started");
    Ok(())
}

/// Keyboard hook: install on first launch, and reinstall after sleep and unlock
/// (or start it, if it failed before). An error goes to the log, no panic: CUE waits for the next attempt.
fn start_hook(tx: &Sender<HotkeyEvent>) {
    match hook::spawn(tx.clone()) {
        Ok(_) => {}
        Err(HookError::AlreadyRunning) => hook::reinstall(),
        Err(e) => tracing::error!("{e}; will retry after sleep or unlock"),
    }
}
