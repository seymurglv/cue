//! Every window of CUE (the Drop, the card, the main and welcome windows) draws through WebView2, whose processes run
//! beside cue.exe. When its browser process ends (it crashed, or something on the computer ended it, as an Explorer
//! restart once did), no window of CUE draws again: the Drop is gone while dictation and the tray icon still work. So
//! each window watches its WebView2: when the browser process is gone, CUE starts again, as "Quit" and a new launch
//! would; when only a page's process ended or hangs, that page loads again. A second loss within a minute of a restart
//! does not restart again (no loop), nor does a loss while Windows signs out or shuts down; the log says to quit and
//! start CUE by hand.
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager, WebviewWindow};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE, ICoreWebView2,
};
use webview2_com::ProcessFailedEventHandler;
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SHUTTINGDOWN};

/// When CUE last started again after a loss of WebView2, in ms since Windows started (`GetTickCount64`: the same clock
/// for every process, never set back): the new process inherits it.
const RESTARTED_AT: &str = "CUE_WEBVIEW_RESTARTED_AT";
/// A loss sooner than this after such a restart does not restart again.
const RESTART_GAP_MS: u64 = 60_000;

/// Set once a restart is asked for or given up: every window reports the same loss, the first one decides.
static RESTARTING: AtomicBool = AtomicBool::new(false);

/// What a failure of WebView2 calls for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// The browser process is gone, and with it every window: CUE starts again.
    Restart,
    /// The browser process is gone again within a minute of the last restart: no further restart.
    GiveUp,
    /// The page's own process ended or hangs: the page loads again.
    Reload,
    /// Left to WebView2 (GPU, utility, a frame CUE does not have, a kind it does not know), or Windows is ending the
    /// session and takes CUE with it.
    Nothing,
}

/// When a failure happens: when CUE last started again after one, now (both on `GetTickCount64`), and whether Windows
/// is signing out or shutting down.
#[derive(Clone, Copy, Debug)]
pub struct Moment {
    pub restarted_at: Option<u64>,
    pub now: u64,
    pub session_ending: bool,
}

/// The recovery for a failure of `kind` at `at`.
pub fn recovery(kind: COREWEBVIEW2_PROCESS_FAILED_KIND, at: Moment) -> Recovery {
    match kind {
        COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED if at.session_ending => Recovery::Nothing,
        COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => {
            if at.restarted_at.is_some_and(|t| at.now.saturating_sub(t) < RESTART_GAP_MS) {
                Recovery::GiveUp
            } else {
                Recovery::Restart
            }
        }
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED
        | COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => Recovery::Reload,
        _ => Recovery::Nothing,
    }
}

/// Whether this CUE was started again after a loss of WebView2 (not by the user or at sign-in).
pub fn restarted() -> bool {
    std::env::var_os(RESTARTED_AT).is_some()
}

fn moment() -> Moment {
    Moment {
        restarted_at: std::env::var(RESTARTED_AT).ok().and_then(|v| v.parse().ok()),
        // SAFETY: neither call takes arguments or touches memory of ours.
        now: unsafe { GetTickCount64() },
        session_ending: unsafe { GetSystemMetrics(SM_SHUTTINGDOWN) } != 0,
    }
}

/// Watches `win`'s WebView2 for the rest of its life. A failure to subscribe is one log line: the window works as before.
pub fn watch(win: &WebviewWindow) {
    let label = win.label().to_string();
    let app = win.app_handle().clone();
    let watched = win.with_webview(move |pw| {
        let name = label.clone();
        let handler = ProcessFailedEventHandler::create(Box::new(move |sender, args| {
            // Without its kind a failure decides nothing: it is no reason to start CUE again.
            let Some(args) = args else { return Ok(()) };
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
            // SAFETY: a call on the event's own arguments, on the thread WebView2 raised it on.
            if unsafe { args.ProcessFailedKind(&mut kind) }.is_err() {
                return Ok(());
            }
            on_failed(&app, &label, kind, sender);
            Ok(())
        }));
        let mut token = 0i64;
        // SAFETY: COM calls on the window's own controller, on the main thread `with_webview` runs this on.
        let added =
            unsafe { pw.controller().CoreWebView2().and_then(|core| core.add_ProcessFailed(&handler, &mut token)) };
        if let Err(e) = added {
            tracing::warn!(window = name, "WebView2 not watched: {e}");
        }
    });
    if let Err(e) = watched {
        tracing::warn!(window = win.label(), "WebView2 not watched: {e}");
    }
}

fn on_failed(app: &AppHandle, label: &str, kind: COREWEBVIEW2_PROCESS_FAILED_KIND, sender: Option<ICoreWebView2>) {
    // The other windows report the loss that already decided a restart (or that it is given up): nothing more to do.
    if RESTARTING.load(Ordering::SeqCst) {
        return;
    }
    let at = moment();
    let what = recovery(kind, at);
    tracing::warn!(window = label, kind = kind.0, session_ending = at.session_ending, ?what, "WebView2 process failed");
    match what {
        Recovery::Restart => {
            RESTARTING.store(true, Ordering::SeqCst);
            // SAFETY: Windows serializes its environment block itself (SetEnvironmentVariableW and the reads of other
            // threads, std's or not, take the same lock), and this runs on the main thread just before the exit. The
            // new process inherits the value.
            unsafe { std::env::set_var(RESTARTED_AT, at.now.to_string()) };
            tracing::warn!("WebView2 is gone: CUE starts again");
            app.request_restart();
        }
        Recovery::GiveUp => {
            RESTARTING.store(true, Ordering::SeqCst);
            tracing::error!(
                "WebView2 is gone again within a minute of a restart: CUE's windows stay blank until it is quit \
                 from the tray and started again"
            );
        }
        Recovery::Reload => {
            // SAFETY: the WebView2 that raised the event, on its own thread.
            let reloaded = match &sender {
                Some(core) => unsafe { core.Reload() }.map_err(|e| e.to_string()),
                None => app.get_webview_window(label).map_or(Ok(()), |w| w.reload().map_err(|e| e.to_string())),
            };
            if let Err(e) = reloaded {
                tracing::warn!(window = label, "page not loaded again: {e}");
            }
        }
        Recovery::Nothing => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED,
    };

    const NOW: u64 = 36_000_000;
    const BROWSER: COREWEBVIEW2_PROCESS_FAILED_KIND = COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED;

    fn at(restarted_at: Option<u64>) -> Moment {
        Moment { restarted_at, now: NOW, session_ending: false }
    }

    #[test]
    fn a_lost_browser_process_starts_cue_again() {
        assert_eq!(recovery(BROWSER, at(None)), Recovery::Restart);
        assert_eq!(recovery(BROWSER, at(Some(NOW - RESTART_GAP_MS))), Recovery::Restart);
    }

    #[test]
    fn a_second_loss_within_a_minute_of_a_restart_does_not_restart_again() {
        assert_eq!(recovery(BROWSER, at(Some(NOW - RESTART_GAP_MS + 1))), Recovery::GiveUp);
        assert_eq!(recovery(BROWSER, at(Some(NOW))), Recovery::GiveUp);
    }

    #[test]
    fn a_loss_while_windows_signs_out_or_shuts_down_starts_nothing() {
        assert_eq!(recovery(BROWSER, Moment { session_ending: true, ..at(None) }), Recovery::Nothing);
    }

    #[test]
    fn a_page_that_ended_or_hangs_loads_again_and_the_rest_is_left_to_webview2() {
        for kind in [
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
        ] {
            assert_eq!(recovery(kind, at(Some(NOW))), Recovery::Reload);
        }
        for kind in [
            COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_KIND(99),
        ] {
            assert_eq!(recovery(kind, at(None)), Recovery::Nothing);
        }
    }
}
