//! Inserting text at the caret: the insert thread (it owns the clipboard operations), the text as it is pasted, and the
//! app the terminal safeguard looks at (which apps are terminals: `cue_platform_win::terminal`).
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};

use cue_core::ask;
use cue_core::flow::{DictationId, InsertOutcome};
use cue_core::refine::join_for_insert;
use cue_platform_win::clipboard::{
    ClipboardApi, PasteResult, WinClipboard, WinKeys, copy_selection, forget_in_history, leave_text, may_paste,
    paste_text_if, spawn_late_guard,
};
use cue_platform_win::focus::{UNKNOWN_APP, WindowRef};
use cue_platform_win::input::{press_right, wait_modifiers_released};
use tokio::sync::mpsc::UnboundedSender;

use super::CoreEvent;

/// A job for the insert thread. `id` is the dictation waiting for the result; `None` is Ctrl+Win+V, "Copy" on the
/// tray card and the answer's Copy: the flow does not wait for their result.
pub enum InsertJob {
    /// There is a field (or it is unknown): paste through the clipboard. `collapse` — "Insert below" (spec 2 §7): first
    /// the Right arrow, which collapses the selection to its end, then the text on a line of its own after it. A
    /// selection that cannot be collapsed is never pasted over: the text stays in the clipboard ("no field"). `target` —
    /// an "Ask" paste: only into that window, the one the question was asked in, and never a text that spans lines (or
    /// starts with a line break) into a terminal (`clipboard::may_paste`), checked right before the Right arrow and the
    /// Ctrl+V; otherwise the text stays in the clipboard ("no field"). `None` — a dictation or Ctrl+Win+V: wherever the
    /// caret is.
    Paste { id: Option<DictationId>, text: String, collapse: bool, target: Option<WindowRef> },
    /// No field: leave the text in the clipboard the normal way (without markers).
    Leave { id: Option<DictationId>, text: String },
    /// The Ctrl+C fallback of "Ask" (spec 2 §6.1): the selection, read through the clipboard, which is put back; the
    /// copy then leaves the Win+V history if Windows allows (decision 27). Only into `target`, the window the selection
    /// was read in at the chord.
    CopySelection { reply: Sender<Option<String>>, target: WindowRef },
}

impl InsertJob {
    fn id(&self) -> Option<DictationId> {
        match self {
            InsertJob::Paste { id, .. } | InsertJob::Leave { id, .. } => *id,
            InsertJob::CopySelection { .. } => None,
        }
    }
}

/// The `cue-insert` thread: jobs one at a time, the result is `CoreEvent::InsertDone`.
///
/// Each job ends within a bounded time: waiting for Ctrl and Win to be released takes up to 5 s, each clipboard open
/// up to 10 attempts of 50 ms (snapshot, write, restore: ≤ ~1.5 s), the pause before restoring the clipboard
/// is 0.5 s; ≤ ~7 s in total, and "Insert below" waits up to `COLLAPSE_WAIT` more for its Right arrow (≤ ~9 s). On top
/// of that the core keeps its own watchdog (`INSERT_WATCHDOG`, 10 s). The Ctrl+C copy of "Ask" takes less: the keys up
/// within 1.5 s, the app's copy within 0.4 s, the clipboard read and put back.
/// `Pasted` is sent from `on_pasted` right after Ctrl+V, before the pause and the clipboard restore
/// (that is the "done" moment).
pub fn spawn_inserter(core: UnboundedSender<CoreEvent>) -> std::io::Result<Sender<InsertJob>> {
    let (tx, rx) = channel::<InsertJob>();
    std::thread::Builder::new().name("cue-insert".into()).spawn(move || {
        let mut clip: Option<WinClipboard> = None;
        for job in rx {
            let id = job.id();
            if catch_unwind(AssertUnwindSafe(|| run(&mut clip, job, &core))).is_err() {
                // Where it crashed is in the log (panic hook); the flow must not wait for the result forever.
                report(&core, id, InsertOutcome::NoField);
            }
        }
    })?;
    Ok(tx)
}

/// How long "Insert below" waits for the modifiers before its Right arrow (Shift held would extend the selection
/// instead): with the paste's own bounds the job still ends within the core's watchdog.
const COLLAPSE_WAIT: Duration = Duration::from_secs(2);

fn report(core: &UnboundedSender<CoreEvent>, id: Option<DictationId>, outcome: InsertOutcome) {
    if let Some(id) = id {
        let _ = core.send(CoreEvent::InsertDone { id, outcome, at: Instant::now() });
    }
}

fn run(clip: &mut Option<WinClipboard>, job: InsertJob, core: &UnboundedSender<CoreEvent>) {
    if clip.is_none() {
        match WinClipboard::new() {
            Ok(c) => *clip = Some(c),
            Err(e) => {
                tracing::warn!("clipboard unavailable: {e}");
                report(core, job.id(), InsertOutcome::NoField);
                return;
            }
        }
    }
    let Some(clip) = clip.as_mut() else { return };
    match job {
        InsertJob::Paste { id, text, collapse, target } => {
            let pasted_text = if collapse { format!("\n{text}") } else { text.clone() };
            let allowed = || target.is_none_or(|t| may_paste(&WinKeys, t, &pasted_text));
            if collapse {
                // Shift must be up first, or Right would extend the selection instead of collapsing it.
                if !wait_modifiers_released(COLLAPSE_WAIT) {
                    tracing::info!("insert below: the selection was not collapsed; text left in the clipboard");
                    leave(clip, &text);
                    report(core, id, InsertOutcome::NoField);
                    return;
                }
                // Not even the Right arrow goes to another window.
                if !allowed() {
                    not_here(clip, &text, core, id);
                    return;
                }
                if !press_right() {
                    tracing::info!("insert below: the selection was not collapsed; text left in the clipboard");
                    leave(clip, &text);
                    report(core, id, InsertOutcome::NoField);
                    return;
                }
            }
            let pasted = paste_text_if(clip, &mut WinKeys, &pasted_text, allowed, || report(core, id, InsertOutcome::Pasted));
            match pasted {
                Ok(PasteResult::Pasted { restored: true }) => {}
                // Pasted; the previous clipboard was not restored (it was changed or not read): not a failure.
                Ok(PasteResult::Pasted { restored: false }) => tracing::info!("insert: previous clipboard not restored"),
                Ok(PasteResult::LeftInClipboard) => {
                    tracing::info!("insert not done (keys held or Ctrl+V not delivered): text left in the clipboard");
                    report(core, id, InsertOutcome::NoField);
                }
                Ok(PasteResult::NotAllowed) => {
                    tracing::info!("ask: the window in front is not the one asked in, or is a terminal: text left in the clipboard");
                    report(core, id, InsertOutcome::NoField);
                }
                Err(e) => {
                    tracing::warn!("insert failed: {e}");
                    if let Err(e) = clip.write_text(&text, false) {
                        tracing::warn!("text not put in the clipboard: {e}");
                    }
                    report(core, id, InsertOutcome::NoField);
                }
            }
        }
        InsertJob::Leave { id, text } => {
            leave(clip, &text);
            report(core, id, InsertOutcome::NoField);
        }
        InsertJob::CopySelection { reply, target } => {
            // A copy too late for this read is still caught, on a thread of its own: the user's clipboard comes back.
            let copy = copy_selection(clip, &mut WinKeys, target, |late| {
                let _ = spawn_late_guard(late);
            });
            if let Some(text) = &copy.text {
                // Detached: the outcome goes to the log as a kind.
                let _ = forget_in_history(text.clone(), copy.since);
            }
            let _ = reply.send(copy.text);
        }
    }
}

/// The text in the clipboard the normal way (without markers), for the user to paste.
fn leave(clip: &mut WinClipboard, text: &str) {
    if let Err(e) = leave_text(clip, text) {
        tracing::warn!("text not put in the clipboard: {e}");
    }
}

/// An "Ask" paste whose window is not in front any more, or is a terminal for its line breaks: nothing is pasted, and
/// the text stays in the clipboard ("no field").
fn not_here(clip: &mut WinClipboard, text: &str, core: &UnboundedSender<CoreEvent>, id: Option<DictationId>) {
    tracing::info!("ask: the window in front is not the one asked in, or is a terminal: text left in the clipboard");
    leave(clip, text);
    report(core, id, InsertOutcome::NoField);
}

/// The app name, if it is a real one: not empty and not the "unknown" placeholder (`UNKNOWN_APP`) from the field query.
pub fn known_app(app: &str) -> Option<&str> {
    let name = app.trim();
    (!name.is_empty() && name != UNKNOWN_APP).then_some(app)
}

/// The first real app in `candidates` (for insert, see `insert_app`; for refine: in focus → at recording start).
/// A late field query must not switch off the terminal safeguard.
/// No real one: `UNKNOWN_APP`.
pub fn first_known_app<'a>(candidates: &[&'a str]) -> &'a str {
    candidates.iter().copied().find(|a| known_app(a).is_some()).unwrap_or(UNKNOWN_APP)
}

/// The app for the terminal safeguard on insert; the candidate order lives here, not at the call site:
/// in focus now → the last known one (`last`) → at the start of recording (`started`; empty for Ctrl+Win+V).
/// Within one dictation the last known app is never older than the app at the start of recording.
pub fn insert_app<'a>(now: &'a str, last: &'a str, started: &'a str) -> &'a str {
    first_known_app(&[now, last, started])
}

/// The text on one line: each line break (`\r\n`, `\n`, `\r` and runs of them) becomes one space;
/// nothing at the start and the end (Enter at the end would run the command); no second space next to a space.
pub fn one_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    // There was a line break after a non-space character: a space goes before the next non-space one.
    let mut gap = false;
    for c in text.chars() {
        if is_line_break(c) {
            gap |= out.chars().next_back().is_some_and(|p| !p.is_whitespace());
            continue;
        }
        if std::mem::take(&mut gap) && !c.is_whitespace() {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// The text as it is pasted: its edges trimmed (`trim_edges`) and the join space added by the text before the caret
/// (`join_for_insert`); over a selection (`replaced`, "Ask"'s Replace), the selection's own edges instead
/// (`ask::replace_edges`), since the text takes its place exactly; on one line in a terminal.
pub fn pasted_text(text: &str, before_cursor: Option<&str>, terminal: bool, replaced: Option<&str>) -> String {
    let text = match replaced {
        Some(selection) => ask::replace_edges(selection, text),
        None => join_for_insert(before_cursor, trim_edges(text)),
    };
    if terminal { one_line(&text) } else { text }
}

/// Edges of the insert: no spaces at the start (a leading line break stays: it is part of the text, e.g. a list item
/// after "List:"), no spaces or line breaks at the end. The only leading space is the join space, which
/// `join_for_insert` adds afterwards.
pub fn trim_edges(text: &str) -> &str {
    text.trim_end().trim_start_matches(|c: char| c.is_whitespace() && !is_line_break(c))
}

/// Characters a text field or a terminal takes as the end of a line.
fn is_line_break(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The flow waits for a paste of its dictation; nothing waits for the copy of a selection (the request task does).
    #[test]
    fn only_a_dictations_paste_or_leave_reports_to_the_flow() {
        let (reply, _rx) = channel();
        let target = WindowRef { window: 0x10, process: 100, app_process: None };
        assert_eq!(InsertJob::CopySelection { reply, target }.id(), None);
        let id = Some(DictationId(3));
        assert_eq!(InsertJob::Paste { id, text: "x".into(), collapse: true, target: Some(target) }.id(), id);
        assert_eq!(InsertJob::Leave { id, text: "x".into() }.id(), id);
    }

    #[test]
    fn placeholder_and_empty_names_are_not_apps() {
        assert_eq!(known_app("Windows Terminal"), Some("Windows Terminal"));
        assert_eq!(known_app(UNKNOWN_APP), None, "the \"unknown\" placeholder from the field query");
        assert_eq!(known_app(""), None);
        assert_eq!(known_app("  "), None);
    }

    #[test]
    fn late_focus_query_falls_back_to_known_app() {
        // Order as on insert: in focus now → last known → at the start of recording.
        // The query on insert was late, so the focus is "unknown"; the last known app is the terminal (the user
        // switched to it during the recording); the recording started in Notepad.
        let app = first_known_app(&[UNKNOWN_APP, "Windows Terminal", "Блокнот"]);
        assert_eq!(app, "Windows Terminal");
        assert!(cue_platform_win::terminal::terminal_app(app), "the terminal safeguard stays on");
        // The last known app is unknown: the app at the start of recording.
        assert_eq!(first_known_app(&[UNKNOWN_APP, "", "Windows Terminal"]), "Windows Terminal");
        // Ctrl+Win+V: no app at the start of recording, so the last known one.
        assert_eq!(first_known_app(&["", "Windows PowerShell", ""]), "Windows PowerShell");
        // A real app now wins over an earlier one.
        assert_eq!(first_known_app(&["Блокнот", "Windows Terminal", "Windows Terminal"]), "Блокнот");
        // Nothing is known.
        assert_eq!(first_known_app(&[UNKNOWN_APP, "", UNKNOWN_APP]), UNKNOWN_APP);
        assert_eq!(first_known_app(&[]), UNKNOWN_APP);
    }

    #[test]
    fn insert_app_prefers_now_then_last_then_started() {
        // The query was late; during the recording the user switched from Notepad to the terminal:
        // the safeguard looks at the terminal.
        assert_eq!(insert_app(UNKNOWN_APP, "Windows Terminal", "Блокнот"), "Windows Terminal");
        assert_eq!(insert_app(UNKNOWN_APP, "Блокнот", "Windows Terminal"), "Блокнот");
        // The last known app is unknown: the app at the start of recording.
        assert_eq!(insert_app(UNKNOWN_APP, "", "Windows Terminal"), "Windows Terminal");
        // The app in focus now comes first.
        assert_eq!(insert_app("Блокнот", "Windows Terminal", "Windows Terminal"), "Блокнот");
        // Ctrl+Win+V: there is no app at the start of recording.
        assert_eq!(insert_app("", "Windows PowerShell", ""), "Windows PowerShell");
        assert_eq!(insert_app(UNKNOWN_APP, "", ""), UNKNOWN_APP);
    }

    #[test]
    fn insert_edges_are_trimmed() {
        assert_eq!(trim_edges(" Привет, как дела?"), "Привет, как дела?");
        assert_eq!(trim_edges("Привет.\n"), "Привет.");
        assert_eq!(trim_edges("  Привет. \r\n \n"), "Привет.");
        assert_eq!(trim_edges("\u{A0}\tПривет"), "Привет", "non-breaking space and tab");
        assert_eq!(trim_edges("\n- пункт"), "\n- пункт", "a leading line break is part of the text");
        assert_eq!(trim_edges(" \n- пункт \n"), "\n- пункт");
        assert_eq!(trim_edges("строка\n  отступ"), "строка\n  отступ", "inside: left alone");
        assert_eq!(trim_edges("без краёв"), "без краёв");
        assert_eq!(trim_edges(" \n "), "");
        assert_eq!(trim_edges(""), "");
    }

    #[test]
    fn only_the_join_space_leads() {
        use cue_core::refine::join_for_insert;
        assert_eq!(join_for_insert(Some("Итак,"), trim_edges("  мы решили. \n")), " мы решили.");
        assert_eq!(join_for_insert(Some("минут.\n"), trim_edges(" Привет, как дела?")), "Привет, как дела?");
        assert_eq!(join_for_insert(None, trim_edges(" Привет ")), "Привет");
        assert_eq!(join_for_insert(Some("Список:"), trim_edges(" \n- пункт")), "\n- пункт");
    }

    /// A replace goes over the selection exactly: two paragraphs selected with the line break after them (a triple click,
    /// Shift+Down) come back as two paragraphs with that line break, and a selection that starts inside a word gets no
    /// join space before it.
    #[test]
    fn a_replace_keeps_its_line_breaks_and_the_selections_edges() {
        let selected = "Первый абзац с ошибкой.\n\nВторой абзац.\n";
        let written = "Первый абзац без ошибки.\n\nВторой абзац.";
        assert_eq!(pasted_text(written, Some("Итак, вот"), false, Some(selected)), "Первый абзац без ошибки.\n\nВторой абзац.\n");
        assert_eq!(pasted_text("слово.\n", Some("пол"), false, Some("слово")), "слово.", "no stray line break or space");
        assert_eq!(pasted_text("Fixed", Some("a"), false, Some("  broken\r\n")), "  Fixed\r\n", "both edges are the selection's");
        // A dictation and Insert keep their rules.
        assert_eq!(pasted_text(" мы решили. \n", Some("Итак,"), false, None), " мы решили.");
        assert_eq!(pasted_text("git status\ngit diff", None, true, None), "git status git diff");
    }

    #[test]
    fn line_breaks_become_single_spaces() {
        assert_eq!(one_line("git status\r\ngit diff"), "git status git diff");
        assert_eq!(one_line("a\nb\rc"), "a b c");
        assert_eq!(one_line("a\n\n\r\nb"), "a b", "a run of line breaks is one space");
        assert_eq!(one_line("a \nb"), "a b", "there is already a space");
        assert_eq!(one_line("a\n b"), "a b");
        assert_eq!(one_line("echo привет\n"), "echo привет", "no Enter at the end");
        assert_eq!(one_line("\r\nls"), "ls");
        assert_eq!(one_line("- один\n- два"), "- один - два");
        assert_eq!(one_line("без переводов"), "без переводов");
        assert_eq!(one_line(""), "");
        assert_eq!(one_line("a\u{2028}b\u{2029}c\u{85}d"), "a b c d", "Unicode line separators are Enter too");
    }
}
