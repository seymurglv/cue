#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod card_window;
mod commands;
mod drop_window;
mod instructions;
mod ipc;
mod lang;
mod logging;
mod main_window;
mod micpreview;
mod paths;
mod runtime;
mod terminal;
mod tray;
mod trayicon;
mod webview_health;
mod welcome_window;

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
        // A second launch opens nothing, but `--welcome` opens the welcome window in the CUE already running.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if welcome_window::asks_welcome(&argv) {
                tracing::info!("welcome window asked for by a second launch");
                welcome_window::open(app);
            }
        }))
        // The parameter matters only on macOS; on Windows autostart is the Run key.
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .manage(drop_window::HitState::default())
        .manage(card_window::CardHits::default())
        .manage(ipc::CoreTx(core_tx))
        .invoke_handler(tauri::generate_handler![
            drop_window::drop_hit_rects,
            drop_window::drop_drag,
            card_window::card_hit_rects,
            ipc::drop_action,
            ipc::drop_ready,
            ipc::card_ready,
            ipc::answer_source,
            commands::settings_get,
            commands::model_get,
            commands::model_retry,
            commands::models_get,
            commands::models_search,
            commands::model_choose,
            commands::model_delete,
            commands::llms_get,
            commands::llms_search,
            commands::llm_choose,
            commands::llm_delete,
            commands::settings_set,
            commands::autostart_get,
            commands::autostart_set,
            commands::history_subscribe,
            commands::history_page,
            commands::stats_get,
            commands::history_search,
            commands::history_copy,
            commands::history_paste,
            commands::history_clear,
            commands::history_audio,
            commands::mic_list,
            commands::mic_preview,
            commands::mic_access,
            commands::welcome_step,
            commands::welcome_close,
            commands::open_link,
            commands::agents_get,
            commands::agent_install_link,
            commands::instructions_get,
            commands::instructions_set,
            commands::instructions_reset,
            commands::instructions_copy,
            commands::instructions_open
        ])
        .setup(move |app| {
            // We log the reason ourselves: Tauri turns a `setup` error into a panic, and the panic hook does not
            // print its message. The app then closes: without the tray icon CUE must not keep running invisibly.
            setup(app, paths, setup_tx, core_rx).inspect_err(|e| tracing::error!("CUE failed to start: {e:#}"))?;
            Ok(())
        })
        .build(tauri::generate_context!());
    match built {
        Ok(app) => app.run(|app, event| match event {
            // Closing the main window never closes CUE: only "Quit" does (`app.exit`, with a code).
            tauri::RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
            tauri::RunEvent::Exit => {
                tracing::info!("CUE is closing");
                if let Some(opencode) = app.try_state::<std::sync::Arc<runtime::opencode::OpenCodeSlot>>() {
                    opencode.rest();
                }
                logging::flush();
            }
            _ => {}
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
    // Interface language: decided here (settings.json first, then Windows); the Drop window and the tray menu both get
    // it, and the core decides it again when the main window changes it.
    let lang = lang::Lang::resolve(settings.ui.language, lang::Lang::detect);
    tracing::info!(lang = lang.code(), setting = ?settings.ui.language, "interface language");
    // Counts only: the terms themselves stay out of the log.
    let terms = cue_core::refine::usable_terms(&settings.dictionary.terms).len();
    tracing::info!(terms, skipped = settings.dictionary.terms.len() - terms, "dictionary");
    // Where the Drop sits: settings.json, then changed only by dragging the Drop (saved by the core).
    let place = settings.drop.position;
    tracing::info!(place = place.code(), "Drop place");
    let ui = ipc::Ui::new(app.handle().clone(), place);
    // New history entries reach the main window over the channel its page opens.
    app.manage(ui.history_feed());
    // Started again after a loss of WebView2 (`webview_health`), CUE keeps running without the Drop if WebView2 is not
    // back yet: dictation and the tray still work, as they did before the restart. A launch by the user stops instead.
    match drop_window::create(app.handle(), lang, place) {
        Ok(win) => {
            let hits = app.state::<drop_window::HitState>().inner().clone();
            drop_window::spawn_hit_test(win.clone(), hits.clone());
            let (drag_tx, drag_rx) = std::sync::mpsc::channel();
            app.manage(drop_window::Placer::new(drag_tx));
            let (tx, hide_tx) = (core_tx.clone(), core_tx.clone());
            drop_window::spawn_placement(
                win,
                ui.clone(),
                place,
                drag_rx,
                hits,
                move |p| {
                    let _ = tx.send(CoreEvent::DropMoved(p));
                },
                move |hidden| {
                    let _ = hide_tx.send(CoreEvent::IdleHidden(hidden));
                },
            );
        }
        Err(e) if webview_health::restarted() => {
            tracing::error!("Drop window not created after a WebView2 restart: {e:#}; quit and start CUE again");
        }
        Err(e) => return Err(e),
    }
    // The "Ask" card at the selection; without it, answers go to the tray.
    let card = match card_window::create(app.handle(), lang) {
        Ok(win) => {
            let hits = app.state::<card_window::CardHits>().inner().0.clone();
            drop_window::spawn_hit_test(win.clone(), hits);
            card_window::spawn(win)
        }
        Err(e) => {
            tracing::error!("card window not created: {e}");
            card_window::Card::none()
        }
    };
    // Without the icon there is no way to quit or pause: if it is not created, CUE does not start (reason in the log).
    tray::create(app.handle(), core_tx.clone(), paths.logs.clone(), lang).context("tray icon not created")?;
    // `--welcome`: the welcome window. Before the hook: the core hears that it holds the hotkey ahead of any chord.
    // `--welcome` is the user's ask at launch: a restart after a loss of WebView2 carries it along, but does not ask again.
    let asked = welcome_window::asks_welcome(std::env::args_os()) && !webview_health::restarted();
    welcome_window::open_at_start(app.handle(), asked);

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
            }
            PowerEvent::Locked | PowerEvent::Suspending => {
                let _ = tx.send(CoreEvent::Input(cue_core::flow::Input::SessionLocked));
            }
        }
    });

    // The main window's commands find CUE's files (the history) here.
    app.manage(paths.clone());
    let rt = runtime::Runtime::new(ui, card, settings, can_save_settings, lang, paths, core_tx);
    // The main window's commands play no recording past the period the core holds.
    app.manage(rt.audio_limit());
    // Quitting stops the OpenCode server CUE started.
    app.manage(rt.opencode());
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
