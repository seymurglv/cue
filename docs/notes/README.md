# Notes: the owner's decisions and where each line of work stands

Read the notes that touch your task before starting it, and update them when a decision is made or the state changes
(date it, `DD.MM.YYYY`). Specs and plans stay in [`docs/specs`](../specs); these notes say what was decided around them
and what is open.

| Note | About |
| --- | --- |
| [repos-and-releases.md](repos-and-releases.md) | this repository, releases, the owner's installed CUE, the future public repo |
| [language-detection.md](language-detection.md) | Russian or English per dictation, never mixed |
| [local-speech-parakeet.md](local-speech-parakeet.md) | speech recognition on this computer: Parakeet Ultra via ONNX |
| [local-cleanup.md](local-cleanup.md) | cleanup on this computer (llama.cpp); the cloud half is replaced by `no-keys` |
| [no-keys.md](no-keys.md) | CUE without keys, cleanup through no AI, this computer or your AI: merged into `main` 09.11.2026 |
| [main-window-parts.md](main-window-parts.md) | the main window, built in parts: status and decisions |
| [optimize-and-settings.md](optimize-and-settings.md) | branch `optimize`, the settings redesign, the no-AI layer: in `main` through `no-keys` |
| [model-picks.md](model-picks.md) | branch `model-picks`, and Task 10 of `no-keys`: which local cleanup models to offer: postponed |
| [macos.md](macos.md) | CUE for macOS, after the model picks |

Open lines of work and their branches (09.11.2026): `model-picks` (pushed, postponed). `no-keys` and the `optimize`
commits it carried were merged into `main` on 09.11.2026. Postponed by the owner: a Linux version for Omarchy (it has
its own dictation; Linear BAN-24). `main` is what ships.