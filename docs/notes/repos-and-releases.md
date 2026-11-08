# Repository and releases

**Since 08.11.2026 there is one repository, `seymurglv/cue` (private).** It was `seymurglv/cue-dev` and holds the full
history, the specs, these notes, `.superpowers` and the releases. The local checkout is still the folder
`Desktop\cue-dev`; its `origin` is `github.com/seymurglv/cue`. The owner wanted one repository so that other editors
(Cursor, Codex, Claude) work in the same place.

Before that (01.11–08.11.2026) there were two: `cue-dev` (history) and `cue` (one clean commit per release, without
`docs/specs`, carrying the installers). That snapshot repository is now `seymurglv/cue-old`, archived (read-only). Its
four releases were recreated here with the same notes and byte-identical installers, each tag on the commit the
snapshot was cut from:

| Tag | Commit | Note |
| --- | --- | --- |
| v0.1.0 | 5ee756b | the snapshot only added the README "Download" link |
| v0.1.1 | 9de3af5 | |
| v0.1.2 | 86de33b | |
| v0.1.3 | 883977c | latest |

## Making a release

1. Bump the version (`Cargo.toml` workspace version, `app/src-tauri/tauri.conf.json`, and `Cargo.lock` with it) in a
   commit `CUE <version>`.
2. Build twice and scan, as README "Build and run" says: `scripts\build-release.ps1 -ScanBuild`, then
   `scripts\build-release.ps1`, then
   `cargo run -p cue-cli -- leak-scan target\dist\release\cue.exe target\dist\scan app\dist`. The computer name equals
   the publisher name `seymurglv`, so the scan reports "computer name no": expected.
3. `gh release create v<version> <installer> --target <full SHA> --title "CUE <version>"` (`--target` needs the full
   SHA).
4. Keep the README "Download" link pointing to `cue/releases/latest`.

## The owner's everyday CUE

Autostart (HKCU Run) launches the installed copy `%LOCALAPPDATA%\Programs\CUE\cue.exe`, not `target\release`. On
04.11.2026 the owner's corrections and habits "didn't work" because the installed 0.1.2 lacked them. After new
features, build the installer with `scripts\build-release.ps1` and install it with `/S` (not published), so the
owner's CUE has them.

## Going public (later)

The owner's plan: when CUE goes public, create a **new** public repository from a clean snapshot of this one: without
`docs/specs`, `docs/notes`, `.superpowers`, `.claude`, `AGENTS.md` and `CLAUDE.md`, one commit, author
`91429742+seymurglv@users.noreply.github.com`, leak-scanned. The owner flips visibility themself. A snapshot can be
built with a temporary index: `read-tree`, `update-index --force-remove` of those paths, `write-tree`, `commit-tree`.
On 05.11.2026 the old snapshot's history was leak-scanned: the hits were public model hashes, made-up test paths and
two harmless test phrases the owner had also dictated.
