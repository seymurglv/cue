//! Which apps are terminals: a line break there is Enter, so a command would run by itself, and Ctrl+C stops the
//! running program. One list for every check — the one-line rule of a paste, where "Ask" writes, and the window looked
//! at right before the Ctrl+C fallback or an "Ask" paste. Pure logic.

/// Terminal names, lowercase: what Windows shows for the app (the description in its exe's properties, which the field
/// query names it by), and the file name of its exe without `.exe` (the name when there is no description, and what the
/// window checks read). The Russian entries are the names Windows shows in Russian. Fluent Terminal is a Store app: its
/// window is a frame of `ApplicationFrameHost`, and the name is the one of the app's own process in it.
const NAMES: &[&str] = &[
    // Windows Terminal and its pseudo-console.
    "windows terminal",
    "windowsterminal",
    "openconsole",
    // PowerShell and the Command Prompt in the console host.
    "windows powershell",
    "powershell",
    "pwsh",
    "командная строка",
    "windows command processor",
    "обработчик команд windows",
    "cmd",
    "cmd.exe",
    "console window host",
    "windows console host",
    "узел окна консоли",
    "conhost",
    "wsl",
    // Git Bash and Cygwin: mintty, whose exe calls itself "Terminal".
    "mintty",
    "terminal",
    "git bash",
    "git-bash",
    "bash",
    // Other terminals and SSH clients.
    "wezterm",
    "wezterm-gui",
    "alacritty",
    "warp",
    "tabby",
    "hyper",
    "termius",
    "windterm",
    "mobaxterm",
    "putty",
    "kitty",
    "conemu",
    "conemu64",
    "conemuc",
    "conemuc64",
    "cmder",
    "fluent terminal",
    "fluentterminal",
    "fluentterminal.app",
];

/// Window classes of terminals: the console host (cmd, PowerShell), Windows Terminal and its pseudo-console, mintty
/// (Git Bash, Cygwin), ConEmu (and Cmder in it), PuTTY, KiTTY, WezTerm, MobaXterm. Terminals built on a browser engine or
/// a cross-platform toolkit (Tabby, Hyper, Termius, Alacritty, WindTerm, Warp) have a class any app may have, and are
/// known by their exe instead.
const CLASSES: &[&str] = &[
    "ConsoleWindowClass",
    "CASCADIA_HOSTING_WINDOW_CLASS",
    "PseudoConsoleWindow",
    "mintty",
    "VirtualConsoleClass",
    "PuTTY",
    "KiTTY",
    "org.wezfurlong.wezterm",
    "TMobaXtermForm",
];

/// The app is a terminal, by the name Windows shows for it or its exe's name, in any case. PowerShell 7 shows its version
/// ("PowerShell 7"), and MobaXterm's portable exe carries its edition and version ("MobaXterm_Personal_24.2").
pub fn terminal_app(app: &str) -> bool {
    let app = app.trim().to_lowercase();
    NAMES.contains(&app.as_str()) || app.starts_with("powershell ") || app.starts_with("mobaxterm_")
}

/// A terminal, by its window class or its executable (`focus::window_kind`): the check right before the Ctrl+C fallback
/// and an "Ask" paste, on the window they would go to, whatever the app was called at the chord.
pub fn terminal_window(class: &str, exe: &str) -> bool {
    CLASSES.iter().any(|c| class.eq_ignore_ascii_case(c)) || terminal_app(exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminals_are_known_by_name() {
        for app in [
            "Windows Terminal",
            "WindowsTerminal",
            "Windows PowerShell",
            "PowerShell",
            "PowerShell 7",
            "pwsh",
            "Командная строка",
            "Windows Command Processor",
            "Обработчик команд Windows",
            "cmd",
            "cmd.exe",
            "Console Window Host",
            "Windows Console Host",
            "  WINDOWS TERMINAL ",
            "mintty",
            "Terminal",
            "Git Bash",
            "WezTerm",
            "Alacritty",
            "Warp",
            "Tabby",
            "Hyper",
            "Termius",
            "WindTerm",
            "MobaXterm",
            "MobaXterm_Personal_24.2",
            "PuTTY",
            "KiTTY",
            "ConEmu",
            "ConEmu64",
            "Cmder",
            "Fluent Terminal",
            "FluentTerminal.App",
            "wsl",
        ] {
            assert!(terminal_app(app), "{app}");
        }
        for app in ["Telegram Desktop", "Visual Studio Code", "Блокнот", "Microsoft Word", "приложение", "", "Terminal Velocity", "Hyperion"] {
            assert!(!terminal_app(app), "{app}");
        }
    }

    /// The window Ctrl+C or a paste would go to, looked at right before: a console of any shell, Windows Terminal,
    /// mintty and the like by their class or executable, whatever the app was called at the chord.
    #[test]
    fn terminal_windows_are_known_by_class_or_executable() {
        for (class, exe) in [
            ("ConsoleWindowClass", "conhost"),
            ("CASCADIA_HOSTING_WINDOW_CLASS", "windowsterminal"),
            ("mintty", "mintty"),
            ("SomethingNew", "pwsh"),
            ("SomethingNew", "PowerShell"),
            ("VirtualConsoleClass", "conemu64"),
            ("Chrome_WidgetWin_1", "wezterm-gui"),
            ("org.wezfurlong.wezterm", ""),
            ("KiTTY", "kitty"),
            ("TMobaXtermForm", "MobaXterm_Personal_24.2"),
            ("Chrome_WidgetWin_1", "Tabby"),
            ("Chrome_WidgetWin_1", "Hyper"),
            ("Chrome_WidgetWin_1", "Termius"),
            ("Window Class", "alacritty"),
            ("Qt5QWindowIcon", "WindTerm"),
            ("", "warp"),
            ("Windows.UI.Core.CoreWindow", "FluentTerminal.App"),
        ] {
            assert!(terminal_window(class, exe), "{class} {exe}");
        }
        let others = [("Chrome_WidgetWin_1", "telegram"), ("Notepad", "notepad"), ("", ""), ("Console", "code"), ("ApplicationFrameWindow", "ApplicationFrameHost")];
        for (class, exe) in others {
            assert!(!terminal_window(class, exe), "{class} {exe}");
        }
    }

    /// Every terminal the window check knows by its exe is one the name check knows too, and the other way round: the
    /// one-line rule and the Ctrl+C rule never disagree about an app.
    #[test]
    fn the_name_and_the_window_checks_agree() {
        for name in NAMES {
            assert!(terminal_app(name) && terminal_window("", name), "{name}");
        }
    }
}
