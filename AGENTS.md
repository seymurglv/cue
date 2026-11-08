# Working on CUE

Instructions for any AI editor or agent working in this repository (Claude Code, Cursor, Codex and others). `CLAUDE.md`
points here, so there is one set of rules.

CUE is voice dictation for Windows: press a hotkey, speak, and clean text appears at the cursor. Rust (Tauri 2) with a
TypeScript + Vite UI, no framework. What it does is in [README.md](README.md); how it works inside is in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Where things are

| Path | What it holds |
| --- | --- |
| `app/crates/core`, `engines`, `platform-win`, `cli` | the Rust crates (see README, "Repository layout") |
| `app/src-tauri` | the app: runtime, windows, tray, installer script |
| `app/ui` | the Drop, its cards, the main and welcome windows |
| `bench` | the comparison bench |
| `docs/ARCHITECTURE.md` | how CUE works inside; keep it true when behaviour changes |
| `docs/specs` | a spec and a plan for every piece of work, by date |
| `docs/notes` | the owner's decisions and the state of each line of work: read the ones that touch your task first |
| `.superpowers` | working material: mockups, research, the release audit, dashboards, and the ledgers of executed plans (`sdd/<plan>/progress.md` lists every ruling made while a plan ran) |
| `scripts` | `build-release.ps1` (the installer), `dev-env.ps1` (tools for llama.cpp's build) |

## Build and check

```powershell
. scripts\dev-env.ps1                 # CMake, Ninja, libclang for llama.cpp, once per shell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cd app; npm install; npm test; npm run build
cd app; npm run tauri dev              # run it, with live reload of the UI
```

The tests need no microphone, network, keys or speech model. Everything must stay green before a commit.

## Rules

- **Talk to the owner in the language of their latest message** (usually Russian). Code, comments, docs, commit
  messages and this repository's files are in English.
- **Spec → plan → code.** New work starts as a spec in `docs/specs` that the owner approves, then a plan, then code.
  Don't build what the owner hasn't approved; ask when a decision is theirs.
- **Commit messages** describe what CUE now does, in plain full sentences, as the history shows. No attribution lines.
- **Privacy first.** API keys live in Windows Credential Manager, never in files, logs or commits. Logs never hold the
  dictated text. The rules are in ARCHITECTURE, "Logging and privacy rules".
- **Previews and scratch files** go outside the repository (your own temp or scratch folder), never into it.
- **One branch per line of work.** `main` is what ships. Open lines of work and their branches are in
  [docs/notes](docs/notes/README.md); update the note when the state changes.
- **The owner's everyday CUE** is the installed copy, `%LOCALAPPDATA%\Programs\CUE\cue.exe` (autostart launches it, not
  `target\release`). After new features, build the installer with `scripts\build-release.ps1` and install it with
  `/S`, so the owner's CUE has them.

## Repository and releases

This repository, `seymurglv/cue` (private), holds everything: the full history, the specs, the notes and the
installer releases. Releases are tagged `v<version>` on the commit they were built from, with the installer
`CUE_<version>_x64-setup.exe` attached. The old snapshot repository is archived as `seymurglv/cue-old`.

When CUE goes public, a **new** repository is created from a clean snapshot of this one (without `docs/specs`,
`docs/notes`, `.superpowers`, `.claude` and `AGENTS.md`/`CLAUDE.md`), leak-scanned, and made public by the owner. See
[docs/notes/repos-and-releases.md](docs/notes/repos-and-releases.md).
