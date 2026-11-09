//! The main window's commands (spec 3 §2–3), and the welcome window's (spec 4a §1). Each one answers the main window,
//! and the welcome window only those its steps need (`guard`): the Drop and the card draw a model's answer, and no page
//! of theirs may read the history or change a setting; the welcome window never reads the history, and of the settings
//! changes the microphone alone. An error is a short code, never text.
//!
//! The history is read here, off the async threads; only the core writes it, and its new entries come over the main
//! window's own channel (`history_subscribe`). An entry's recording is read by its id alone, checked to be an entry
//! id, never a path from the page, and comes as bytes (`history_audio`). The settings are read and changed through the
//! core, which owns them (`CoreEvent::Settings`). Start at sign-in is the autostart plugin's Run key, as in the tray
//! menu, whose check follows within 5 s. The microphones are listed as Windows names them, and a preview sends the page that started it a microphone's level alone (`micpreview`). The welcome window opens its links
//! by name, never by an address of its page's (`open_link`).
use std::path::Path;
use std::time::Duration;

use cue_core::flow::Input;
use cue_core::history;
use cue_core::model::{ModelState, ModelsView};
use cue_core::settings::SettingChange;
use cue_core::stats::{self, Stats};
use cue_engines::hub;
use cue_platform_win::{focus, mic};
use tauri::ipc::Channel;
use tauri::{AppHandle, State, WebviewWindow};
use tauri_plugin_autostart::ManagerExt;

use crate::ipc::{CoreTx, DayView, EntryView, HistoryFeed, MAIN_LABEL, PageView, SettingsView, WELCOME_LABEL};
use crate::micpreview;
use crate::paths::Paths;
use crate::runtime::recordings::{self, AudioLimit, Playable};
use crate::runtime::{Choice, CoreEvent, MIC_SETTINGS, SettingsCmd, local_today, open_url, utc_offset_minutes};

/// How long a command waits for the core's answer about the settings.
const CORE_WAIT: Duration = Duration::from_secs(3);
/// How long a search of Hugging Face may take, its repositories' files asked for included.
const MODELS_SEARCH_WAIT: Duration = Duration::from_secs(30);
/// The longest search text sent.
const MODELS_QUERY_CHARS: usize = 100;
/// "Show earlier": this many more days with entries.
const EARLIER_DAYS: usize = 7;
/// The most entries a search finds.
const SEARCH_LIMIT: usize = 200;
/// How long "Paste again" waits for the window under the main one to come in front.
const BRING_TIMEOUT: Duration = Duration::from_millis(500);
/// The commands the welcome window's page may call: what its steps need — the language, the speech model, the
/// microphone (of the settings it changes the microphone alone: `may_change`) — and its own (`WELCOME_ONLY`).
const WELCOME_COMMANDS: [&str; 10] = [
    "settings_get",
    "model_get",
    "model_retry",
    "settings_set",
    "mic_list",
    "mic_preview",
    "mic_access",
    "welcome_step",
    "welcome_close",
    "open_link",
];
/// The welcome window's own commands: no other window calls them.
const WELCOME_ONLY: [&str; 3] = ["welcome_step", "welcome_close", "open_link"];

/// What went wrong, as a code (camelCase in JSON).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CmdError {
    /// Not the main window, or the welcome window asking for what is not its own.
    NotMain,
    /// The core did not answer in time (or is not running).
    Core,
    /// No such history entry: cleared, or past the history's period, since the window showed it. For a recording: none
    /// to play — never made, cleared, or past its period.
    NotFound,
    /// A setting change the settings do not know, a link CUE does not have, or a recording asked for by a name that is
    /// no entry id.
    BadValue,
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

/// Where the welcome window's links go, by the names its page gives (`open_link`): the Windows microphone settings.
fn link(target: &str) -> Option<&'static str> {
    match target {
        "micPrivacy" => Some(MIC_SETTINGS),
        _ => None,
    }
}

/// Whether Windows lets CUE use the microphone (camelCase in JSON): the welcome window tells a refusal before any
/// level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MicAccess {
    Allowed,
    Denied,
}

/// Every command starts here, with its name: the main window's page is answered for all but the welcome window's own,
/// the welcome window's for `WELCOME_COMMANDS`, no other.
fn guard(window: &WebviewWindow, command: &str) -> Result<(), CmdError> {
    guard_label(window.label(), command)
        .inspect_err(|_| tracing::warn!(window = window.label(), command, "window command refused"))
}

fn guard_label(label: &str, command: &str) -> Result<(), CmdError> {
    let answered = match label {
        MAIN_LABEL => !WELCOME_ONLY.contains(&command),
        WELCOME_LABEL => WELCOME_COMMANDS.contains(&command),
        _ => false,
    };
    if answered { Ok(()) } else { Err(CmdError::NotMain) }
}

/// The main window changes any setting; the welcome window, the microphone alone.
fn may_change(label: &str, change: &SettingChange) -> bool {
    label == MAIN_LABEL || (label == WELCOME_LABEL && matches!(change, SettingChange::MicDevice(_)))
}

/// The speech model's state (spec "Local speech recognition"): ready, downloading with its bytes, failed.
#[tauri::command]
pub async fn model_get(window: WebviewWindow, core: State<'_, CoreTx>) -> Result<ModelState, CmdError> {
    guard(&window, "model_get")?;
    let (reply, answer) = tokio::sync::oneshot::channel();
    send(&core, CoreEvent::ModelGet(reply))?;
    match tokio::time::timeout(CORE_WAIT, answer).await {
        Ok(Ok(state)) => Ok(state),
        _ => Err(CmdError::Core),
    }
}

/// "Retry" on the speech model: its download goes on from where it stopped. The new state comes as `cue://model`.
#[tauri::command]
pub fn model_retry(window: WebviewWindow, core: State<'_, CoreTx>) -> Result<(), CmdError> {
    guard(&window, "model_retry")?;
    send(&core, CoreEvent::ModelRetry)
}

/// The Model page's list (spec "Choosing the speech model"): the short list and the found models here, each with its
/// state. Changes come as `cue://models`.
#[tauri::command]
pub async fn models_get(window: WebviewWindow, core: State<'_, CoreTx>) -> Result<ModelsView, CmdError> {
    guard(&window, "models_get")?;
    let (reply, answer) = tokio::sync::oneshot::channel();
    send(&core, CoreEvent::ModelsGet(reply))?;
    match tokio::time::timeout(CORE_WAIT, answer).await {
        Ok(Ok(view)) => Ok(view),
        _ => Err(CmdError::Core),
    }
}

/// Hugging Face searched: ONNX repositories by downloads, each with whether CUE can run it. `Failed` — no answer (no
/// connection, an error of the service).
#[tauri::command]
pub async fn models_search(window: WebviewWindow, core: State<'_, CoreTx>, query: String) -> Result<Vec<hub::Found>, CmdError> {
    guard(&window, "models_search")?;
    let query: String = query.chars().take(MODELS_QUERY_CHARS).collect();
    let (reply, answer) = tokio::sync::oneshot::channel();
    send(&core, CoreEvent::ModelsSearch { query, reply })?;
    match tokio::time::timeout(MODELS_SEARCH_WAIT, answer).await {
        Ok(Ok(Ok(found))) => Ok(found),
        Ok(Ok(Err(e))) => {
            tracing::info!("models: the search failed: {e}");
            Err(CmdError::Failed)
        }
        _ => Err(CmdError::Core),
    }
}

/// `owner/name` of a Hugging Face repository: letters, digits, `-`, `_` and `.`, neither part starting with a dot.
fn repository(repo: &str) -> bool {
    let part = |p: &str| !p.is_empty() && !p.starts_with('.') && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    repo.split_once('/').is_some_and(|(owner, name)| part(owner) && part(name))
}

/// A model chosen on the Model page: `id` from its list, or `repo` from the search. The switch's states come as
/// `cue://models`.
#[tauri::command]
pub fn model_choose(window: WebviewWindow, core: State<'_, CoreTx>, id: Option<String>, repo: Option<String>) -> Result<(), CmdError> {
    guard(&window, "model_choose")?;
    let choice = match (id, repo) {
        (Some(id), None) => Choice::Id(id),
        (None, Some(repo)) if repository(&repo) => Choice::Repo(repo),
        _ => return Err(CmdError::BadValue),
    };
    send(&core, CoreEvent::ModelChoose(choice))
}

/// The Cleanup page's local list: the short list, found models downloaded here, a switch on its way.
#[tauri::command]
pub async fn llms_get(window: WebviewWindow, core: State<'_, CoreTx>) -> Result<ModelsView, CmdError> {
    guard(&window, "llms_get")?;
    let (reply, answer) = tokio::sync::oneshot::channel();
    send(&core, CoreEvent::LlmsGet(reply))?;
    match tokio::time::timeout(CORE_WAIT, answer).await {
        Ok(Ok(view)) => Ok(view),
        _ => Err(CmdError::Core),
    }
}

/// Hugging Face searched for GGUF models by downloads, each with whether CUE can run it.
#[tauri::command]
pub async fn llms_search(window: WebviewWindow, core: State<'_, CoreTx>, query: String) -> Result<Vec<hub::LlmFound>, CmdError> {
    guard(&window, "llms_search")?;
    let query: String = query.chars().take(MODELS_QUERY_CHARS).collect();
    let (reply, answer) = tokio::sync::oneshot::channel();
    send(&core, CoreEvent::LlmsSearch { query, reply })?;
    match tokio::time::timeout(MODELS_SEARCH_WAIT, answer).await {
        Ok(Ok(Ok(found))) => Ok(found),
        Ok(Ok(Err(e))) => {
            tracing::info!("language models: the search failed: {e}");
            Err(CmdError::Failed)
        }
        _ => Err(CmdError::Core),
    }
}

/// A local model chosen on the Cleanup page: `id` from its list, or `repo` from the search.
#[tauri::command]
pub fn llm_choose(window: WebviewWindow, core: State<'_, CoreTx>, id: Option<String>, repo: Option<String>) -> Result<(), CmdError> {
    guard(&window, "llm_choose")?;
    let choice = match (id, repo) {
        (Some(id), None) => Choice::Id(id),
        (None, Some(repo)) if repository(&repo) => Choice::Repo(repo),
        _ => return Err(CmdError::BadValue),
    };
    send(&core, CoreEvent::LlmChoose(choice))
}

/// A local model's file deleted from the Cleanup page: `false` — the one in use, one being switched to, or one CUE
/// does not know.
#[tauri::command]
pub async fn llm_delete(window: WebviewWindow, core: State<'_, CoreTx>, id: String) -> Result<bool, CmdError> {
    guard(&window, "llm_delete")?;
    let (reply, answer) = tokio::sync::oneshot::channel();
    send(&core, CoreEvent::LlmDelete { id, reply })?;
    match tokio::time::timeout(CORE_WAIT, answer).await {
        Ok(Ok(deleted)) => Ok(deleted),
        _ => Err(CmdError::Core),
    }
}

/// A model's files deleted from the Model page: `false` — the model in use, one being switched to, or one CUE does not
/// know.
#[tauri::command]
pub async fn model_delete(window: WebviewWindow, core: State<'_, CoreTx>, id: String) -> Result<bool, CmdError> {
    guard(&window, "model_delete")?;
    let (reply, answer) = tokio::sync::oneshot::channel();
    send(&core, CoreEvent::ModelDelete { id, reply })?;
    match tokio::time::timeout(CORE_WAIT, answer).await {
        Ok(Ok(deleted)) => Ok(deleted),
        _ => Err(CmdError::Core),
    }
}

/// The settings as the core holds them (spec 3 §3).
#[tauri::command]
pub async fn settings_get(window: WebviewWindow, core: State<'_, CoreTx>) -> Result<SettingsView, CmdError> {
    guard(&window, "settings_get")?;
    ask_core(&core, SettingsCmd::Get).await
}

/// One setting changed (`{"kind": "micDevice", "value": "…"}`, as `SettingChange` reads it): the core takes it and
/// writes settings.json, and answers with the settings as they are now — unchanged when it refused the value. From the
/// welcome window, the microphone alone (`may_change`).
#[tauri::command]
pub async fn settings_set(
    window: WebviewWindow,
    change: serde_json::Value,
    core: State<'_, CoreTx>,
) -> Result<SettingsView, CmdError> {
    guard(&window, "settings_set")?;
    let change: SettingChange = serde_json::from_value(change).map_err(|_| CmdError::BadValue)?;
    if !may_change(window.label(), &change) {
        tracing::warn!(window = window.label(), "setting change refused");
        return Err(CmdError::NotMain);
    }
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
    guard(&window, "autostart_get")?;
    app.autolaunch().is_enabled().map_err(|e| {
        tracing::warn!("autostart: state not read: {e}");
        CmdError::Autostart
    })
}

/// Start at sign-in on or off; the answer is the Run key as it is afterwards. A failure is one log line, and the answer
/// shows the key unchanged. The tray menu's check follows within 5 s.
#[tauri::command]
pub async fn autostart_set(window: WebviewWindow, on: bool, app: AppHandle) -> Result<bool, CmdError> {
    guard(&window, "autostart_set")?;
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
    limit: State<'_, AudioLimit>,
) -> Result<PageView, CmdError> {
    guard(&window, "history_page")?;
    let (dir, audio, limit) = (paths.history.clone(), paths.history_audio.clone(), limit.get());
    blocking(move || {
        let today = local_today();
        let playable = Playable::read(&audio, limit, &today);
        page(&dir, before.as_deref(), &today, &|id: &str| playable.has(id))
    })
    .await
}

/// Today's numbers, the main window's first section, counted from usage.csv; no file yet is all zeros.
#[tauri::command]
pub async fn stats_get(window: WebviewWindow, paths: State<'_, Paths>) -> Result<Stats, CmdError> {
    guard(&window, "stats_get")?;
    let path = paths.usage.clone();
    blocking(move || {
        let csv = std::fs::read_to_string(&path).unwrap_or_default();
        stats::compute(&csv, &local_today(), utc_offset_minutes())
    })
    .await
}

/// New history entries, as they come, over `channel` (spec 3 §2): only the page that opened it gets them. A new
/// subscription (the page loaded again) replaces the old one; the window's end drops it (`main_window`).
#[tauri::command]
pub async fn history_subscribe(
    window: WebviewWindow,
    channel: Channel<EntryView>,
    feed: State<'_, HistoryFeed>,
) -> Result<(), CmdError> {
    guard(&window, "history_subscribe")?;
    feed.follow(channel);
    Ok(())
}

/// The first page (`before` is `None`) is today and yesterday (`today` is `YYYY-MM-DD`); any other, up to
/// `EARLIER_DAYS` days with entries before the day `before`. `has_audio`: whether an entry's recording plays.
fn page(dir: &Path, before: Option<&str>, today: &str, has_audio: &impl Fn(&str) -> bool) -> PageView {
    let page = match before {
        None => history::recent(dir, today),
        Some(before) => history::older(dir, before, EARLIER_DAYS),
    };
    PageView::of(&page, has_audio)
}

/// The entries whose text, question or text as it was heard holds `query`, newest first, at most `SEARCH_LIMIT`.
#[tauri::command]
pub async fn history_search(
    window: WebviewWindow,
    query: String,
    paths: State<'_, Paths>,
    limit: State<'_, AudioLimit>,
) -> Result<Vec<DayView>, CmdError> {
    guard(&window, "history_search")?;
    let (dir, audio, limit) = (paths.history.clone(), paths.history_audio.clone(), limit.get());
    blocking(move || {
        let playable = Playable::read(&audio, limit, &local_today());
        let has_audio = |id: &str| playable.has(id);
        history::search(&dir, &query, SEARCH_LIMIT).iter().map(|d| DayView::of(d, &has_audio)).collect()
    })
    .await
}

/// The recording of the entry `id`, as the bytes of its Ogg Opus file (an `ArrayBuffer` in the page, not JSON), for
/// the page to play: `NotFound` when there is none to play — never made (an entry of an earlier version, recordings off,
/// an attempt whose audio could not be written), still being encoded, cleared, or past its period even before the next
/// prune deletes it. `id` must be an entry id (`history::audio_file`): never a path from the page.
#[tauri::command]
pub async fn history_audio(
    window: WebviewWindow,
    id: String,
    paths: State<'_, Paths>,
    limit: State<'_, AudioLimit>,
) -> Result<tauri::ipc::Response, CmdError> {
    guard(&window, "history_audio")?;
    let file = recording_file(&id)?;
    let (dir, limit) = (paths.history_audio.clone(), limit.get());
    match blocking(move || recordings::read(&dir, &file, limit, &local_today())).await? {
        Ok(Some(bytes)) => Ok(tauri::ipc::Response::new(bytes)),
        Ok(None) => Err(CmdError::NotFound),
        Err(e) => {
            tracing::warn!("history: recording not read: {e}");
            Err(CmdError::Failed)
        }
    }
}

/// The name of the recording of the entry `id` in the recordings folder; anything but an entry id is refused, and the
/// log never gets what the page sent.
fn recording_file(id: &str) -> Result<String, CmdError> {
    history::audio_file(id).ok_or_else(|| {
        tracing::warn!(chars = id.chars().count(), "history: a recording asked for by a name that is no entry id");
        CmdError::BadValue
    })
}

/// "Copy": the entry's text into the clipboard.
#[tauri::command]
pub async fn history_copy(
    window: WebviewWindow,
    id: String,
    paths: State<'_, Paths>,
    core: State<'_, CoreTx>,
) -> Result<(), CmdError> {
    guard(&window, "history_copy")?;
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
    guard(&window, "history_paste")?;
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
    guard(&window, "history_clear")?;
    send(&core, CoreEvent::HistoryClear)
}

/// The text of the entry `id`.
async fn entry_text(dir: &Path, id: String) -> Result<String, CmdError> {
    let dir = dir.to_owned();
    blocking(move || history::find(&dir, &id)).await?.map(|e| e.text).ok_or(CmdError::NotFound)
}

/// The microphones Windows has now, as it names them, and the one it records from by default (spec 3 §3). `Debug` shows
/// how many, never a name.
#[derive(Clone, PartialEq, Eq, serde::Serialize)]
pub struct MicList {
    names: Vec<String>,
    default: Option<String>,
}

impl std::fmt::Debug for MicList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MicList").field("names", &self.names.len()).field("default", &self.default.is_some()).finish()
    }
}

/// The microphone list for the settings: the devices are listed off the async threads.
#[tauri::command]
pub async fn mic_list(window: WebviewWindow) -> Result<MicList, CmdError> {
    guard(&window, "mic_list")?;
    blocking(|| MicList { names: mic::input_names(), default: mic::default_name() }).await
}

/// The live level of a microphone (`device`, by its name; `None` or blank — the Windows one) while the page shows it
/// (`on`), as `cue://mic-level` to this window alone, and `cue://mic-failed` when the microphone fails; `on: false`
/// ends it. The level only: the audio goes nowhere. The preview is this window's: another window's "off" or end leaves
/// it (`micpreview`). Not `async` on purpose: Tauri runs it on the main thread, in the order the page sent them and in
/// turn with the window's end (`main_window`, `welcome_window`), so an "on" and an "off" never cross and no preview
/// outlives the window. It only starts threads.
#[tauri::command]
pub fn mic_preview(
    window: WebviewWindow,
    on: bool,
    device: Option<String>,
    app: AppHandle,
) -> Result<(), CmdError> {
    guard(&window, "mic_preview")?;
    if on {
        micpreview::start(&app, window.label(), device.filter(|d| !d.trim().is_empty()));
    } else {
        micpreview::stop(window.label());
    }
    Ok(())
}

/// Whether Windows lets CUE use the microphone, from its privacy settings (`mic::access_denied`): read, nothing opened.
#[tauri::command]
pub async fn mic_access(window: WebviewWindow) -> Result<MicAccess, CmdError> {
    guard(&window, "mic_access")?;
    let access = if blocking(mic::access_denied).await? { MicAccess::Denied } else { MicAccess::Allowed };
    tracing::info!(?access, "microphone access");
    Ok(access)
}

/// The welcome window's step (spec 4a §1), told on every change and when its page loads: on its first dictation's
/// step (`dictation`) the hotkey works, on any other it starts nothing (`Input::WelcomeGate`). Not `async` on purpose,
/// as `mic_preview`: the steps reach the core in the order the page took them.
#[tauri::command]
pub fn welcome_step(window: WebviewWindow, dictation: bool, core: State<'_, CoreTx>) -> Result<(), CmdError> {
    guard(&window, "welcome_step")?;
    tracing::info!(dictation, "welcome window step");
    send(&core, CoreEvent::Input(Input::WelcomeGate(!dictation)))
}

/// "Done": the welcome window closes, and the hotkey works again as it goes (`welcome_window`).
#[tauri::command]
pub async fn welcome_close(window: WebviewWindow) -> Result<(), CmdError> {
    guard(&window, "welcome_close")?;
    window.close().map_err(|e| {
        tracing::warn!("welcome window not closed: {e}");
        CmdError::Failed
    })
}

/// One of the welcome window's links (`link`): the page names it, the address is CUE's own. It opens in the default
/// browser or in Windows Settings, from a short thread of its own, so the shell never holds a command.
#[tauri::command]
pub async fn open_link(window: WebviewWindow, target: String) -> Result<(), CmdError> {
    guard(&window, "open_link")?;
    let url = link(&target).ok_or(CmdError::BadValue)?;
    let spawned = std::thread::Builder::new().name("cue-open-link".into()).spawn(move || {
        if let Err(e) = open_url(url) {
            tracing::warn!(link = target.as_str(), "link did not open: {e}");
        }
    });
    match spawned {
        Ok(_) => Ok(()),
        Err(e) => {
            tracing::warn!("link not opened: {e}");
            Err(CmdError::Failed)
        }
    }
}

fn send(core: &CoreTx, ev: CoreEvent) -> Result<(), CmdError> {
    core.0.send(ev).map_err(|_| CmdError::Core)
}

/// File and system work (the history, the microphones), off the async threads.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, CmdError> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|_| CmdError::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cue_core::history::{Entry, Mode};
    use serde_json::json;

    /// Every command a window's page calls (`main`'s handler list, but for the Drop's and the card's own).
    const COMMANDS: [&str; 28] = [
        "settings_get",
        "model_get",
        "model_retry",
        "models_get",
        "models_search",
        "model_choose",
        "model_delete",
        "llms_get",
        "llms_search",
        "llm_choose",
        "llm_delete",
        "stats_get",
        "settings_set",
        "autostart_get",
        "autostart_set",
        "history_subscribe",
        "history_page",
        "history_search",
        "history_copy",
        "history_paste",
        "history_clear",
        "history_audio",
        "mic_list",
        "mic_preview",
        "mic_access",
        "welcome_step",
        "welcome_close",
        "open_link",
    ];

    /// No other window's page gets an answer: the Drop's and the card's draw what a model wrote.
    #[test]
    fn guard_refuses_other_windows() {
        assert_eq!(guard_label(MAIN_LABEL, "settings_get"), Ok(()));
        for label in ["drop", "card", "Main", "", "main ", "Welcome", "welcome "] {
            for command in COMMANDS {
                assert_eq!(guard_label(label, command), Err(CmdError::NotMain), "{label:?} {command}");
            }
        }
        // Errors and answers are short codes.
        assert_eq!(serde_json::to_value(CmdError::NotMain).unwrap(), json!("notMain"));
        assert_eq!(serde_json::to_value(CmdError::BadValue).unwrap(), json!("badValue"));
        assert_eq!(serde_json::to_value(PasteAgain::Pasted).unwrap(), json!("pasted"));
        assert_eq!(serde_json::to_value(PasteAgain::Clipboard).unwrap(), json!("clipboard"));
        let access = [MicAccess::Allowed, MicAccess::Denied];
        assert_eq!(serde_json::to_value(access).unwrap(), json!(["allowed", "denied"]));
    }

    /// The welcome window gets what its steps need — the language, the speech model, the microphone — and its own
    /// commands, never the history or start at sign-in; its own are no other window's. No window has a key command.
    #[test]
    fn welcome_label_may_call_only_its_commands() {
        let settings_and_model = ["settings_get", "settings_set", "model_get", "model_retry"];
        let microphone = ["mic_list", "mic_preview", "mic_access"];
        let own = ["welcome_step", "welcome_close", "open_link"];
        for command in COMMANDS {
            let welcome = [&settings_and_model[..], &microphone, &own].iter().any(|its| its.contains(&command));
            assert_eq!(guard_label(WELCOME_LABEL, command).is_ok(), welcome, "{command}");
            assert_eq!(guard_label(MAIN_LABEL, command).is_ok(), !own.contains(&command), "{command}");
        }
        for command in ["history_page", "history_subscribe", "history_paste", "autostart_set", "drop_action", ""] {
            assert_eq!(guard_label(WELCOME_LABEL, command), Err(CmdError::NotMain), "{command:?}");
        }
        for gone in ["key_state", "key_set", "key_check"] {
            assert!(!COMMANDS.contains(&gone) && !WELCOME_COMMANDS.contains(&gone), "{gone}");
        }
    }

    /// A repository from the search names an owner and a repository, nothing else: never a path.
    #[test]
    fn a_chosen_repository_is_owner_and_name() {
        for repo in ["istupakov/parakeet-tdt-0.6b-v3-onnx", "myned-ai/Parakeet_TDT.v3"] {
            assert!(repository(repo), "{repo}");
        }
        for repo in ["", "parakeet", "a/b/c", "../x", "a/..", "a/b c", "a/b?x=1", "/b", "a/", r"a", "a/.git"] {
            assert!(!repository(repo), "{repo:?}");
        }
    }

    /// A recording plays in the main window alone, and only by an entry id: anything else — a path, a recording's own
    /// file name — is refused before the folder is looked at.
    #[test]
    fn history_audio_answers_the_main_window_and_entry_ids_alone() {
        assert_eq!(guard_label(MAIN_LABEL, "history_audio"), Ok(()));
        for label in [WELCOME_LABEL, "drop", "card", "Main", ""] {
            assert_eq!(guard_label(label, "history_audio"), Err(CmdError::NotMain), "{label:?}");
        }
        assert_eq!(recording_file("2026-10-31T07:30:00.123-4").as_deref(), Ok("2026-10-31T07-30-00.123-4.ogg"));
        for id in [
            "",
            "..",
            r"..\..\settings.json",
            r"C:\Windows\win.ini",
            "2026-10-31T07-30-00.123-4.ogg",
            "2026-10-31T07:30:00.123-4/../x",
            "2026-10-31T07:30:00.123-0",
            "2026-10-31T07:30:00.123",
        ] {
            assert_eq!(recording_file(id), Err(CmdError::BadValue), "{id:?}");
        }
        assert_eq!(serde_json::to_value(CmdError::NotFound).unwrap(), json!("notFound"));
    }

    /// Of the settings, as its page sends a change, the welcome window changes the microphone alone; the main window,
    /// any.
    #[test]
    fn welcome_may_set_only_the_microphone() {
        let change = |v: serde_json::Value| serde_json::from_value::<SettingChange>(v).unwrap();
        let mic = [
            change(json!({ "kind": "micDevice", "value": "USB Microphone" })),
            change(json!({ "kind": "micDevice", "value": null })),
        ];
        let others = [
            change(json!({ "kind": "historyKeep", "value": "off" })),
            change(json!({ "kind": "historyAudio", "value": "off" })),
            change(json!({ "kind": "historyAudio", "value": "day" })),
            change(json!({ "kind": "translateTarget", "value": "de" })),
            change(json!({ "kind": "uiLanguage", "value": "ru" })),
        ];
        for c in &mic {
            assert!(may_change(WELCOME_LABEL, c) && may_change(MAIN_LABEL, c));
        }
        for (i, c) in others.iter().enumerate() {
            assert!(!may_change(WELCOME_LABEL, c), "change {i}");
            assert!(may_change(MAIN_LABEL, c), "change {i}");
        }
        assert!(mic.iter().chain(&others).all(|c| !may_change("drop", c)));
    }

    /// The page names a link, CUE has the address — the Windows microphone settings; nothing else opens, an address
    /// least of all, nor a page for a key.
    #[test]
    fn open_link_accepts_only_known_targets() {
        assert_eq!(link("soniox"), None, "the Soniox key is gone");
        assert_eq!(link("anthropic"), None, "CUE asks for no key");
        assert_eq!(link("micPrivacy"), Some("ms-settings:privacy-microphone"));
        for other in [
            "https://console.soniox.com",
            "ms-settings:privacy-microphone",
            "Soniox",
            "soniox ",
            "micprivacy",
            "openrouter",
            "calc.exe",
            "javascript:alert(1)",
            "",
        ] {
            assert_eq!(link(other), None, "{other:?}");
        }
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cue-test-{}-commands-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn entry(ts: &str, text: &str) -> Entry {
        let (id, app) = (format!("{ts}-1"), "Notepad".into());
        Entry { id, ts: ts.into(), mode: Mode::Dictation, app, text: text.into(), question: None, raw: None, habits: Vec::new() }
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
        // Today's entry has a recording to play.
        let none = |_: &str| false;
        let today = |id: &str| id.starts_with("2026-10-31");
        let first = page(&dir, None, "2026-10-31", &today);
        assert_eq!(first.days.iter().map(|d| d.date.as_str()).collect::<Vec<_>>(), ["2026-10-31", "2026-10-30"]);
        assert!(first.more);
        let shown = serde_json::to_value(&first).unwrap();
        assert_eq!(shown["days"][0]["entries"][0]["time"], json!("09:15"));
        assert_eq!(shown["days"][0]["entries"][0]["text"], json!("Hello."));
        assert_eq!(shown["days"][0]["entries"][0]["hasAudio"], json!(true));
        assert_eq!(shown["days"][1]["entries"][0]["hasAudio"], json!(false));

        let earlier = page(&dir, Some("2026-10-30"), "2026-10-31", &none);
        let dates: Vec<&str> = earlier.days.iter().map(|d| d.date.as_str()).collect();
        let week = ["2026-10-28", "2026-10-20", "2026-10-19", "2026-10-18", "2026-10-17", "2026-10-16", "2026-10-15"];
        assert_eq!(dates, week);
        assert!(earlier.more, "2026-10-01 is still there");
        let last = page(&dir, Some("2026-10-15"), "2026-10-31", &none);
        assert_eq!(last.days.len(), 1);
        assert!(!last.more);

        // Neither today nor yesterday has entries: none on the first page, the rest behind "Show earlier".
        let quiet = page(&dir, None, "2026-11-05", &none);
        assert!(quiet.days.is_empty() && quiet.more);
        let empty = page(&tmp("pages-none"), None, "2026-10-31", &none);
        assert!(empty.days.is_empty() && !empty.more);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The names go to the window; the log would get only how many.
    #[test]
    fn mic_list_names_reach_the_window_but_not_the_log() {
        let names = vec!["Headset of Jane".into(), "Microphone (USB)".into()];
        let list = MicList { names, default: Some("Headset of Jane".into()) };
        let expected = json!({ "names": ["Headset of Jane", "Microphone (USB)"], "default": "Headset of Jane" });
        assert_eq!(serde_json::to_value(&list).unwrap(), expected);
        let shown = format!("{list:?}");
        assert!(!shown.contains("Jane") && !shown.contains("USB"), "{shown}");
        let none = MicList { names: Vec::new(), default: None };
        assert_eq!(serde_json::to_value(none).unwrap(), json!({ "names": [], "default": null }));
    }


}
