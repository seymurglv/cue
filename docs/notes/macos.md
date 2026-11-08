# CUE for macOS (after the model picks)

The owner plans a macOS version after the Windows work. Their Mac: MacBook Pro M2, 16 GB RAM, 256 GB disk. Said
05.11.2026: no Apple Developer account for now; a person must be able to take the app, install it, grant permissions
and run it fully local and private.

Plan for an unsigned build (ad hoc or a free self-signed identity): Gatekeeper's "Open Anyway" once, and keep the
Accessibility, Input Monitoring and Microphone grants across updates with a stable self-signed identity (ad hoc
signatures change per build and reset the grants). Default to local speech + local cleanup (llama.cpp with Metal);
"Ask" needs Anthropic, so it is off or optional in a fully local setup. The Windows-only pieces and their macOS
counterparts: dashboard https://claude.ai/artifact/KsSGy6k4HSUtJyHXhm7DfS (and `.superpowers/dashboard`).
