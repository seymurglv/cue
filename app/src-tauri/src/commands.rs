//! The main window's commands (spec 3 §2–3). Each one answers the main window alone (`guard`): the Drop and the card
//! draw a model's answer, and no page of theirs may read the history, change a setting or touch a key. An error is a
//! short code, never text.
//!
//! The history is read here, off the async threads; only the core writes it, and its new entries come over the main
//! window's own channel (`history_subscribe`). The settings are read and changed through the core, which owns them
//! (`CoreEvent::Settings`). Start at sign-in is the autostart plugin's Run key, as in the tray menu, whose check follows
//! within 5 s. Keys are read and saved in Windows Credential Manager, where `cue-cli keys` keeps them: a key never
//! leaves this module — not in a reply, an error or the log — only its mask does; a check is a free probe request
//! (`cue_engines::check`), never a paid one.
use std::path::Path;
use std::time::Duration;

use cue_core::history;
use cue_core::settings::SettingChange;
use cue_engines::EngineError;
use cue_engines::keys::{self, KeyName};
use cue_platform_win::focus;
use tauri::ipc::Channel;
use tauri::{AppHandle, State, WebviewWindow};
use tauri_plugin_autostart::ManagerExt;

use crate::ipc::{CoreTx, DayView, EntryView, HistoryFeed, MAIN_LABEL, PageView, SettingsView};
use crate::paths::Paths;
use crate::runtime::{CoreEvent, SettingsCmd, local_today};

/// How long a command waits for the core's answer about the settings.
const CORE_WAIT: Duration = Duration::from_secs(3);
/// "Show earlier": this many more days with entries.
const EARLIER_DAYS: usize = 7;
/// The most entries a search finds.
const SEARCH_LIMIT: usize = 200;
/// A key check gives up after this, as `cue-cli keys check` does.
const KEY_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
/// How long "Paste again" waits for the window under the main one to come in front.
const BRING_TIMEOUT: Duration = Duration::from_millis(500);

/// What went wrong, as a code (camelCase in JSON).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CmdError {
    /// Not the main window.
    NotMain,
    /// The core did not answer in time (or is not running).
    Core,
    /// No such history entry: cleared, or past the history's period, since the window showed it.
    NotFound,
    /// A setting change the settings do not know.
    BadValue,
    /// A key other than `soniox` and `anthropic`.
    BadName,
    /// An empty key is not saved.
    EmptyKey,
    /// Windows Credential Manager failed.
    Store,
    /// The Run key of start at sign-in could not be read.
    Autostart,
    /// The command's work crashed.
    Failed,
}

/// Where "Paste again" put the text (camelCase in JSON).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PasteAgain {
    /// Into the window that was under the main one, now in front (the insert thread checks it once more right before
    /// Ctrl+V and otherwise leaves the text in the clipboard).
    Pasted,
    /// No such window: into the clipboard.
    Clipboard,
}

/// A key as the window shows it: whether one is saved, and its mask (`keys::mask`), never the key itself.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct KeyView {
    saved: bool,
    mask: String,
}

impl KeyView {
    fn of(key: Option<&str>) -> Self {
        Self { saved: key.is_some(), mask: key.map(keys::mask).unwrap_or_default() }
    }
}

/// What a key check found (camelCase in JSON): the key works; the service refused it (key, balance or limit); the check
/// did not get through (network, timeout, the service); no key is saved; Windows Credential Manager failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyCheck {
    Works,
    BadKey,
    Unchecked,
    Missing,
    Store,
}

/// Every command starts here: only the main window's page is answered.
fn guard(window: &WebviewWindow) -> Result<(), CmdError> {
    guard_label(window.label()).inspect_err(|_| tracing::warn!(window = window.label(), "main window command refused"))
}

fn guard_label(label: &str) -> Result<(), CmdError> {
    if label == MAIN_LABEL { Ok(()) } else { Err(CmdError::NotMain) }
}

/// The settings as the core holds them (spec 3 §3).
#[tauri::command]
pub async fn settings_get(window: WebviewWindow, core: State<'_, CoreTx>) -> Result<SettingsView, CmdError> {
    guard(&window)?;
    ask_core(&core, SettingsCmd::Get).await
}

/// One setting changed (`{"kind": "micDevice", "value": "…"}`, as `SettingChange` reads it): the core takes it and
/// writes settings.json, and answers with the settings as they are now — unchanged when it refused the value.
#[tauri::command]
pub async fn settings_set(
    window: WebviewWindow,
    change: serde_json::Value,
    core: State<'_, CoreTx>,
) -> Result<SettingsView, CmdError> {
    guard(&window)?;
    let change: SettingChange = serde_json::from_value(change).map_err(|_| CmdError::BadValue)?;
    ask_core(&core, SettingsCmd::Set(change)).await
}

async fn ask_core(core: &CoreTx, cmd: SettingsCmd) -> Result<SettingsView, CmdError> {
    let (reply, answer) = tokio::sync::oneshot::channel();
    send(core, CoreEvent::Settings { cmd, reply })?;
    match tokio::time::timeout(CORE_WAIT, answer).await {
        Ok(Ok(view)) => Ok(view),
        _ => Err(CmdError::Core),
    }
}

/// Whether CUE starts at sign-in: the Run key as it is.
#[tauri::command]
pub async fn autostart_get(window: WebviewWindow, app: AppHandle) -> Result<bool, CmdError> {
    guard(&window)?;
    app.autolaunch().is_enabled().map_err(|e| {
        tracing::warn!("autostart: state not read: {e}");
        CmdError::Autostart
    })
}

/// Start at sign-in on or off; the answer is the Run key as it is afterwards. A failure is one log line, and the answer
/// shows the key unchanged. The tray menu's check follows within 5 s.
#[tauri::command]
pub async fn autostart_set(window: WebviewWindow, on: bool, app: AppHandle) -> Result<bool, CmdError> {
    guard(&window)?;
    let m = app.autolaunch();
    // Only a change: removing a value that is not there is an error.
    let set = m.is_enabled().and_then(|now| match (now, on) {
        (false, true) => m.enable(),
        (true, false) => m.disable(),
        _ => Ok(()),
    });
    match set {
        Ok(()) => tracing::info!(on, "start at sign-in set from the main window"),
        Err(e) => tracing::warn!(on, "start at sign-in not set: {e}"),
    }
    m.is_enabled().map_err(|_| CmdError::Autostart)
}

/// A page of the history (spec 3 §2): today and yesterday first (`before` is `None`), then "Show earlier" — the days
/// with entries before the day `before`.
#[tauri::command]
pub async fn history_page(
    window: WebviewWindow,
    before: Option<String>,
    paths: State<'_, Paths>,
) -> Result<PageView, CmdError> {
    guard(&window)?;
    let dir = paths.history.clone();
    blocking(move || page(&dir, before.as_deref(), &local_today())).await
}

/// New history entries, as they come, over `channel` (spec 3 §2): only the page that opened it gets them. A new
/// subscription (the page loaded again) replaces the old one; the window's end drops it (`main_window`).
#[tauri::command]
pub async fn history_subscribe(
    window: WebviewWindow,
    channel: Channel<EntryView>,
    feed: State<'_, HistoryFeed>,
) -> Result<(), CmdError> {
    guard(&window)?;
    feed.follow(channel);
    Ok(())
}

/// The first page (`before` is `None`) is today and yesterday (`today` is `YYYY-MM-DD`); any other, up to
/// `EARLIER_DAYS` days with entries before the day `before`.
fn page(dir: &Path, before: Option<&str>, today: &str) -> PageView {
    let page = match before {
        None => history::recent(dir, today),
        Some(before) => history::older(dir, before, EARLIER_DAYS),
    };
    PageView::from(&page)
}

/// The entries whose text or question holds `query`, newest first, at most `SEARCH_LIMIT`.
#[tauri::command]
pub async fn history_search(
    window: WebviewWindow,
    query: String,
    paths: State<'_, Paths>,
) -> Result<Vec<DayView>, CmdError> {
    guard(&window)?;
    let dir = paths.history.clone();
    blocking(move || history::search(&dir, &query, SEARCH_LIMIT).iter().map(DayView::from).collect()).await
}

/// "Copy": the entry's text into the clipboard.
#[tauri::command]
pub async fn history_copy(
    window: WebviewWindow,
    id: String,
    paths: State<'_, Paths>,
    core: State<'_, CoreTx>,
) -> Result<(), CmdError> {
    guard(&window)?;
    let text = entry_text(&paths.history, id).await?;
    send(&core, CoreEvent::HistoryText { text, target: None })
}

/// "Paste again" (decision 3): into the window right under the main one — not CUE's, visible, not minimized, not a tool
/// window, not the desktop — once it has come in front; with no such window, or one that would not come forward, into
/// the clipboard. The last result (Ctrl+Win+V) stays as it is.
#[tauri::command]
pub async fn history_paste(
    window: WebviewWindow,
    id: String,
    paths: State<'_, Paths>,
    core: State<'_, CoreTx>,
) -> Result<PasteAgain, CmdError> {
    guard(&window)?;
    let text = entry_text(&paths.history, id).await?;
    // An HWND is not `Send`: its number goes to the walk.
    let own = window.hwnd().map(|h| h.0 as isize);
    let target = match own {
        Ok(own) => {
            let below = move || focus::window_below(own).filter(|&w| focus::bring_forward(w, BRING_TIMEOUT));
            blocking(below).await.ok().flatten()
        }
        Err(e) => {
            tracing::warn!("main window: no HWND: {e}");
            None
        }
    };
    let pasted = if target.is_some() { PasteAgain::Pasted } else { PasteAgain::Clipboard };
    send(&core, CoreEvent::HistoryText { text, target })?;
    tracing::info!(to = ?pasted, "history: paste again");
    Ok(pasted)
}

/// "Clear history" (the window asks twice): the core deletes every day.
#[tauri::command]
pub async fn history_clear(window: WebviewWindow, core: State<'_, CoreTx>) -> Result<(), CmdError> {
    guard(&window)?;
    send(&core, CoreEvent::HistoryClear)
}

/// The text of the entry `id`.
async fn entry_text(dir: &Path, id: String) -> Result<String, CmdError> {
    let dir = dir.to_owned();
    blocking(move || history::find(&dir, &id)).await?.map(|e| e.text).ok_or(CmdError::NotFound)
}

/// Whether a key is saved, and its mask.
#[tauri::command]
pub async fn key_state(window: WebviewWindow, name: String) -> Result<KeyView, CmdError> {
    guard(&window)?;
    let name = key_name(&name)?;
    match blocking(move || keys::lookup(name)).await? {
        Ok(key) => Ok(KeyView::of(key.as_deref())),
        Err(e) => {
            tracing::warn!("{} key: {e}", name.as_str());
            Err(CmdError::Store)
        }
    }
}

/// Saves a key pasted in the window (`keys::set` trims it and refuses an empty one); the answer is its mask.
#[tauri::command]
pub async fn key_set(window: WebviewWindow, name: String, value: String) -> Result<KeyView, CmdError> {
    guard(&window)?;
    let name = key_name(&name)?;
    let saved = blocking(move || keys::set(name, &value).map(|()| KeyView::of(Some(value.trim())))).await?;
    match saved {
        Ok(view) => {
            tracing::info!(key = name.as_str(), "key saved from the main window");
            Ok(view)
        }
        Err(EngineError::KeyProblem(_)) => Err(CmdError::EmptyKey),
        Err(e) => {
            tracing::warn!("{} key not saved: {e}", name.as_str());
            Err(CmdError::Store)
        }
    }
}

/// Checks a saved key with a free probe request, as `cue-cli keys check` does.
#[tauri::command]
pub async fn key_check(window: WebviewWindow, name: String) -> Result<KeyCheck, CmdError> {
    guard(&window)?;
    let name = key_name(&name)?;
    let probed = match blocking(move || keys::lookup(name)).await? {
        Ok(Some(key)) => Ok(Some(probe(name, &key).await)),
        Ok(None) => Ok(None),
        Err(e) => {
            tracing::warn!("{} key: {e}", name.as_str());
            Err(e)
        }
    };
    let check = key_check_of(&probed);
    tracing::info!(key = name.as_str(), ?check, "key checked from the main window");
    Ok(check)
}

/// The probe of `name`'s service with `key`; the error never holds the key (`cue_engines::check`).
async fn probe(name: KeyName, key: &str) -> Result<(), EngineError> {
    let http = reqwest::Client::builder()
        .timeout(KEY_CHECK_TIMEOUT)
        .build()
        .map_err(|e| EngineError::Network(e.to_string()))?;
    match name {
        KeyName::Soniox => cue_engines::check::soniox(&http, key).await,
        KeyName::Anthropic => cue_engines::check::anthropic(&http, key).await,
        KeyName::OpenRouter => cue_engines::check::openrouter(&http, key).await,
    }
}

/// The store read and, with a key, the probe → what the window shows.
fn key_check_of(probed: &Result<Option<Result<(), EngineError>>, EngineError>) -> KeyCheck {
    match probed {
        Ok(Some(Ok(()))) => KeyCheck::Works,
        Ok(Some(Err(e))) if e.is_key_problem() => KeyCheck::BadKey,
        Ok(Some(Err(_))) => KeyCheck::Unchecked,
        Ok(None) => KeyCheck::Missing,
        Err(_) => KeyCheck::Store,
    }
}

/// The keys the window shows: Soniox and Anthropic.
fn key_name(name: &str) -> Result<KeyName, CmdError> {
    match KeyName::parse(name) {
        Some(k @ (KeyName::Soniox | KeyName::Anthropic)) => Ok(k),
        _ => Err(CmdError::BadName),
    }
}

fn send(core: &CoreTx, ev: CoreEvent) -> Result<(), CmdError> {
    core.0.send(ev).map_err(|_| CmdError::Core)
}

/// File and Credential Manager work, off the async threads.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, CmdError> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|_| CmdError::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cue_core::history::{Entry, Mode};
    use serde_json::json;

    /// Only the main window's page gets an answer: the Drop's and the card's draw what a model wrote.
    #[test]
    fn guard_refuses_other_windows() {
        assert_eq!(guard_label(MAIN_LABEL), Ok(()));
        for label in ["drop", "card", "Main", "", "main "] {
            assert_eq!(guard_label(label), Err(CmdError::NotMain), "{label:?}");
        }
        // Errors and answers are short codes.
        assert_eq!(serde_json::to_value(CmdError::NotMain).unwrap(), json!("notMain"));
        assert_eq!(serde_json::to_value(CmdError::EmptyKey).unwrap(), json!("emptyKey"));
        assert_eq!(serde_json::to_value(PasteAgain::Pasted).unwrap(), json!("pasted"));
        assert_eq!(serde_json::to_value(PasteAgain::Clipboard).unwrap(), json!("clipboard"));
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cue-test-{}-commands-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn entry(ts: &str, text: &str) -> Entry {
        let (id, app) = (format!("{ts}-1"), "Notepad".into());
        Entry { id, ts: ts.into(), mode: Mode::Dictation, app, text: text.into(), question: None }
    }

    /// The window opens on today and yesterday; "Show earlier" brings the days with entries before the last one shown.
    #[test]
    fn first_history_page_is_today_and_yesterday() {
        let dir = tmp("pages");
        for date in ["2026-10-31", "2026-10-30", "2026-10-28", "2026-10-20", "2026-10-19", "2026-10-18", "2026-10-17"] {
            history::append(&dir, &entry(&format!("{date}T09:15:00.000"), "Hello.")).unwrap();
        }
        for date in ["2026-10-16", "2026-10-15", "2026-10-01"] {
            history::append(&dir, &entry(&format!("{date}T09:15:00.000"), "Earlier.")).unwrap();
        }
        let first = page(&dir, None, "2026-10-31");
        assert_eq!(first.days.iter().map(|d| d.date.as_str()).collect::<Vec<_>>(), ["2026-10-31", "2026-10-30"]);
        assert!(first.more);
        let shown = serde_json::to_value(&first).unwrap();
        assert_eq!(shown["days"][0]["entries"][0]["time"], json!("09:15"));
        assert_eq!(shown["days"][0]["entries"][0]["text"], json!("Hello."));

        let earlier = page(&dir, Some("2026-10-30"), "2026-10-31");
        let dates: Vec<&str> = earlier.days.iter().map(|d| d.date.as_str()).collect();
        let week = ["2026-10-28", "2026-10-20", "2026-10-19", "2026-10-18", "2026-10-17", "2026-10-16", "2026-10-15"];
        assert_eq!(dates, week);
        assert!(earlier.more, "2026-10-01 is still there");
        let last = page(&dir, Some("2026-10-15"), "2026-10-31");
        assert_eq!(last.days.len(), 1);
        assert!(!last.more);

        // Neither today nor yesterday has entries: none on the first page, the rest behind "Show earlier".
        let quiet = page(&dir, None, "2026-11-05");
        assert!(quiet.days.is_empty() && quiet.more);
        let empty = page(&tmp("pages-none"), None, "2026-10-31");
        assert!(empty.days.is_empty() && !empty.more);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// As `cue-cli keys check` tells them apart: a refusal is the key's fault; a network, a timeout or a failing
    /// service leave it unchecked.
    #[test]
    fn key_check_maps_engine_errors() {
        let probed = |r: Result<(), EngineError>| key_check_of(&Ok(Some(r)));
        assert_eq!(probed(Ok(())), KeyCheck::Works);
        assert_eq!(probed(Err(EngineError::KeyProblem("401: invalid x-api-key".into()))), KeyCheck::BadKey);
        assert_eq!(probed(Err(EngineError::from_status(402, "balance"))), KeyCheck::BadKey);
        assert_eq!(probed(Err(EngineError::Timeout)), KeyCheck::Unchecked);
        assert_eq!(probed(Err(EngineError::Network("dns".into()))), KeyCheck::Unchecked);
        assert_eq!(probed(Err(EngineError::from_status(503, "overloaded"))), KeyCheck::Unchecked);
        assert_eq!(probed(Err(EngineError::Protocol("404: not found".into()))), KeyCheck::Unchecked);
        assert_eq!(key_check_of(&Ok(None)), KeyCheck::Missing);
        assert_eq!(key_check_of(&Err(EngineError::Service("could not read the soniox key".into()))), KeyCheck::Store);
        let codes = [KeyCheck::Works, KeyCheck::BadKey, KeyCheck::Unchecked, KeyCheck::Missing, KeyCheck::Store];
        assert_eq!(serde_json::to_value(codes).unwrap(), json!(["works", "badKey", "unchecked", "missing", "store"]));
    }

    /// Only the mask leaves: in the reply, in `Debug`.
    #[test]
    fn key_view_never_holds_the_value() {
        let key = "sk-ant-api03-SECRETMIDDLEPART-0123-f3a9";
        let view = KeyView::of(Some(key));
        let shown = [serde_json::to_string(&view).unwrap(), format!("{view:?}")];
        for s in &shown {
            assert!(!s.contains("SECRET") && !s.contains(key), "{s}");
        }
        assert_eq!(serde_json::to_value(&view).unwrap(), json!({ "saved": true, "mask": "sk-ant…f3a9" }));
        let short = KeyView::of(Some("short-key-123"));
        assert_eq!(serde_json::to_value(&short).unwrap(), json!({ "saved": true, "mask": "•".repeat(13) }));
        assert_eq!(serde_json::to_value(KeyView::of(None)).unwrap(), json!({ "saved": false, "mask": "" }));
    }

    #[test]
    fn the_window_shows_only_soniox_and_anthropic_keys() {
        assert_eq!(key_name("soniox"), Ok(KeyName::Soniox));
        assert_eq!(key_name("anthropic"), Ok(KeyName::Anthropic));
        assert_eq!(key_name("openrouter"), Err(CmdError::BadName));
        assert_eq!(key_name("openai"), Err(CmdError::BadName));
    }
}
