# CUE without keys: cleanup, translation and Ask through your AI or this computer (design)

Date: 2026-11-08. Status: for the owner's approval. Branch `no-keys` (from `optimize`). Mockup (the owner's picks applied):
https://claude.ai/artifact/UAKqQmWV7WoD2EyuP6SSdK. Supersedes the parked drafts `2026-11-05-cleanup-settings-design.md`
and the model catalog of `2026-11-05-cleanup-model-picks.md` (branch `model-picks`); builds on
`2026-11-05-no-ai-layer-design.md`, which this work implements as written (option A, exact matches only).

Marked **(my call)**: decisions the owner did not make and a builder may revisit without asking. Everything else is the
owner's.

## Why

The owner wants CUE simple and practical, and points at Hex (github.com/anomalyco/hex, by the OpenCode team): Hex holds
no keys; its AI rewrite goes through the user's own OpenCode, and any failure pastes the text as said. CUE today asks
for an API key, offers eight cloud providers with key checks and balances, and "Ask" works only with an Anthropic key.
The owner's words: speech recognition and cleanup "turned into a big mess"; the settings must be practical for people.

## Decisions (the owner's, 08.11.2026)

1. **CUE holds no API keys at all.** Cleanup, translation and Ask go through one of: no AI; a model on this computer; or
   an AI agent the user already has installed and signed in to. The eight cloud providers, the key fields, Check and the
   balance probes go away.
2. **The page is variant A**: on top a three-part control **Без ИИ · Этот компьютер · Ваш AI** (No AI · This computer ·
   Your AI); below it, the list for the chosen part.
3. **Your AI**: **OpenCode is the default and comes first**; then Codex, then Grok. **Claude Code is not offered at all**:
   Anthropic's terms forbid third-party apps routing requests through a user's Claude subscription. Gemini / Antigravity
   is not offered either (Google treats it the same way). Someone who wants Claude uses it inside OpenCode with an
   Anthropic API key of their own, configured in OpenCode, not in CUE.
4. **This computer**: the user always sees and chooses which model downloads; CUE never downloads a model by itself.
   Recommended models are marked and say why; any other GGUF can be found on Hugging Face, as today.
5. **No auto-pick with hidden fallback** (variant C was rejected for that); only the one fallback the user chooses: "If
   the AI didn't answer: paste as said / clean with the model on this computer".
6. **Instructions like Hex**: one plain file the user edits by hand or through an AI agent ("Изменить с AI").
7. **Models are chosen by their published characteristics, not measured** (cloud and local alike).
8. **Ask and translation also go through the chosen route**, so no key is left anywhere.

## Goals

- A user with OpenCode installed gets cleanup that works with no setup at all; a user with nothing installed sees, on one
  page, exactly what each option is, what it costs in disk and where text goes, and chooses in one press.
- One place for each choice; nothing hidden behind "Advanced".
- Every failure says what failed in one word on the Drop and never loses the dictation.

## Non-goals

- Claude Code, Gemini CLI / Antigravity, any other agent beyond OpenCode, Codex and Grok (new ones can be added later as
  rows of the same list).
- Editing CUE's other settings through an agent (Hex's "Edit with agent" for all settings): later. Only the instructions
  now.
- Streaming the cleanup while recording; macOS (`docs/notes/macos.md`) — later.
- Styles per app beyond what habits already learn.

## Anti-goals (a failure even if it works)

- CUE storing, reading or asking for any API key or login token.
- A model downloaded without the user pressing on it.
- An OpenCode server left running by CUE after CUE quits, rests or the session locks.
- A dictated sentence acting on the computer: an agent run with tools, file access or a shell it could use.
- The transcript visible in a process list (passed as a command-line argument) or written to a temp file.
- A setting reachable from two places; a status said by color alone.

## The design

Copy is given in Russian (the owner's language) with the English meaning; every string exists in both languages.

### First settings page

"Чистка текста" (Cleanup) shows what cleans now: `OpenCode · <model>`, `Codex · GPT-6 Luna`, `Grok · <model>`, the local
model's name, "Без ИИ", or "Не выбрано" (nothing chosen yet). While a model downloads: its percent in Bebas and the 2 px
line under the row, as the speech model's row does today. When the chosen agent cannot be used: the value in amber,
`<Agent>: войдите заново` (sign in again) or `<Agent> не найден` (not found). The Keys page and every key row are gone.

### Cleanup page (Чистка текста)

1. **The three-part control**: Без ИИ · Этот компьютер · Ваш AI.
2. **Без ИИ**: one row explaining what happens without a model: dictionary and learned spellings, no "эм"/"ээ", tidy
   spaces, instant (the no-AI layer).
3. **Этот компьютер**:
   - Group **"Для ваших языков · русский, английский"**: the recommended models (see Models below). The languages in its
     heading are the ones found in the user's history (the model-picks idea: recommend by language, not hardware); with
     no history yet, the interface language.
   - Group **"Другие"**: the rest of the catalog and any downloaded found model; last row **"Найти модель на Hugging
     Face"** (today's search, `hub::search_llms`), its results marked "CUE не проверял" (not checked by CUE), models CUE
     cannot run shown pale with the reason, as today.
   - Each row: name; a line with "Рекомендуем" (when it is), languages, speed, size. A model not downloaded shows its size
     on the right; pressing it starts the download in place (percent, bar, "N из M МБ"; pressing again stops it). The one
     in use has the check; a downloaded one has "Удалить" (delete) on hover, the one in use too, as today.
   - **Speed**: "~2 с" from the model's published characteristics until CUE has cleaned with it; then the median of the
     last 10 cleanups of that model on this computer, "у вас 1,8 с" (yours: 1.8 s). Kept in
     `%LOCALAPPDATA%\CUE\speed.json`, per model id **(my call)**.
4. **Ваш AI**:
   - Group "Найдено на этом компьютере": OpenCode, Codex, Grok, always in that order. A found agent: its line (how it
     signs in, a typical time); the one in use has the check. Not found: pale, "Не установлен", and a button: OpenCode
     "Установить" (opens opencode.ai's install page), the others "Как установить". OpenCode's row carries "Рекомендуем".
   - **Модель** (Model): a dropdown of the models the chosen agent offers. Defaults by characteristics (fast and cheap is
     enough for cleanup): OpenCode "Быстрая · авто" (see below), Codex GPT-6 Luna, Grok its fast model. Remembered per
     agent.
   - **Если AI не ответил** (if the AI didn't answer): "Вставить как сказано" (paste as said, after the no-AI layer) or
     "Почистить: <local model>" — offered only when a local model is downloaded. Default: paste as said.
   - Grok keeps every run in its own history (`~/.grok/sessions`) and has no switch to stop it: Grok's line says so in a
     few words ("Grok хранит диктовки в своей истории") **(my call: be honest rather than hide it)**.
5. **Ваши инструкции** (hidden on Без ИИ): opens the Instructions page. Its line: "Стандартные · можно поправить через AI"
   or "Изменены вами".
6. Footer, one line on where text goes:
   - OpenCode: "Текст уходит туда, куда настроен OpenCode: ваш провайдер, ваш ключ или подписка. CUE ключей не хранит."
   - Codex / Grok: "Текст уходит в ChatGPT через Codex, по вашему входу. CUE не хранит ключи." (Grok alike.)
   - This computer: "Бесплатно. Текст не покидает этот компьютер, модель работает на процессоре."
   - No AI: "Текст вставляется как распознан. CUE сам правит слова из словаря и убирает «эм» и «ээ»."
   - Nothing chosen: "Пока ... не выбран(а), текст вставляется без ИИ."

### Instructions page (Ваши инструкции)

- The instructions are one file, `%APPDATA%\CUE\instructions.md`, shown in a text box on the page and editable there.
  It holds only CUE's style rules in plain words — today's `BOLD_PROMPT`, or the user's `refine.custom` text. The fixed
  protocol stays in code and is never in the file: `SYSTEM_PROMPT` (output format, never answering the text, the context
  tags), the dictionary and habits blocks, the translate instruction. No placeholders in the file.
- The default file is written in the interface language (Russian or English) **(my call)**; both texts live in code, so
  "Вернуть стандартные" (restore the standard ones) rewrites it.
- **"Изменить с AI"** opens a small menu:
  - "Открыть в <agent>": starts the chosen agent (OpenCode, else Codex, else Grok) interactively in a new terminal window
    in `%APPDATA%\CUE\`, with a first message that is the task below.
  - "Скопировать задание": copies the task for any AI.
  - The task (Russian or English): edit only `instructions.md`; read `AGENTS.md` first; ask the user what to change;
    after saving, ask the user to look at CUE's Instructions page, which shows before and after on their last
    dictation; never touch keys, sign-ins or other CUE settings.
- `%APPDATA%\CUE\AGENTS.md`: written by CUE when missing or older than CUE's version **(my call: CUE owns it)**. It says
  what the file is for, what the agent may change (style rules), what not (the protocol is not in the file anyway;
  settings.json, Credential Manager, the history), and how to test.
- CUE watches the file and takes a saved change at the next dictation (debounced 300 ms). The page shows "Изменены вами
  · только что" and "было → стало" on the last dictation of the history (re-cleaned once with the new instructions on
  the current route; skipped on Без ИИ). An empty or unreadable file means the standard instructions, with an amber line
  on the page.
- CUE keeps the previous version as `instructions.prev.md` each time the file changes. There is no `cue-cli
  instructions test` **(my call, after the builder check)**: `cue-cli` is not in the installer and would load a second
  model; the page's before/after on the last dictation is the check, for the user and for the agent.
- **Habit rules on an agent route use CUE's template, never the agent (my call)**: the rule's 5 s wait would expire on
  most agent runs, and Grok would keep every habit example in its history. On this computer they keep using the local
  model as today.
- Whatever the user signed in to inside OpenCode (an API key or a provider's subscription) is the user's own setup with
  that provider; CUE does not inspect it **(my call)**. Claude Code is left out because CUE itself would be the third
  party driving a Claude subscription.

### Your AI: how CUE talks to each agent

Common to all three:
- Run with no window, the working folder an empty `%LOCALAPPDATA%\CUE\agent\`, the transcript and context on stdin
  (never as an argument), tools, web access and file writes off, output as JSON; CUE parses `{"text": …}` from the
  answer as it does today and runs the same output checks (`settle`, `sanity_ok_with`, `drop_copied_context`,
  `decide_translation`).
- The prompt is the same system text as the cloud path today (`claude::system` blocks joined, as `chat::system_text`
  does), so habits, dictionary and translation work unchanged.
- **Only an agent that can be fully stripped is offered** (this follows from the anti-goal; settled after the builder
  check): on the installed version, its flags must leave it no shell, no file reading and no network tool for cleanup
  and translation. The selection and the text before the caret come from other apps (a web page, an email), so a
  read-only sandbox that still runs shell commands is not enough. An agent that cannot be stripped is left out of the
  list for that version, and the builder reports it.
- Timeout **(my call)**: Codex and Grok start a process on every request, so agents get their own budget, `10 s + 1 s
  per 100 words`, plus a 5 s start allowance on the first request after CUE starts or the agent rests; OpenCode, a warm
  server, has the same budget (planned as today's `refine::timeout_ms`, but OpenCode 2.2.5 took 5–8 s for a 165-word
  cleanup on 09.11.2026, past that 4 s). Ask's first words through an agent: 15 s (20 s with
  search). The page says "не ответил вовремя", not a number. The local fallback is used only when the local model is
  loaded at that moment; a resting one is not woken for a fallback (it would add up to 15 s), and the text is pasted as
  said.
- Failure kinds: not installed; not signed in or no access (the CLI's own exit code or message); timed out; other. Each
  maps to the fallback the user chose and to a Drop word (below).
- Each agent's exact flags and endpoints must be checked against the installed version before building (the research of
  08.11.2026 found: claude 2.1.318 not used, codex-cli 0.166.0, grok 1.0.34; OpenCode not installed here).

**OpenCode** (default):
- Found as `opencode` on PATH or in its usual install folders, else as OpenCode Desktop's own CLI
  (`%APPDATA%\ai.opencode.desktop\cli\<version>-<arch>\opencode-cli.exe`, the highest version), else by a running server
  it announces in its state file (`~\.local\state\opencode\service.json`, loopback only): then CUE uses that server and
  never starts one.
- If an OpenCode server is already running and announces itself in OpenCode's own state file (loopback URL only), CUE
  uses it. Otherwise CUE starts `opencode serve` on 127.0.0.1 on a free port when cleanup first needs it, and stops it
  after the same 15 minutes without a dictation, on session lock, on sleep and when CUE quits — the rest rules of the
  local models (`runtime/llm.rs`). A server CUE did not start is never stopped by CUE.
- One stateless generate call where the server offers it; otherwise a session created for the request and deleted after
  it, so dictations do not pile up in OpenCode's history.
- "Быстрая · авто": CUE picks from the models OpenCode lists the first whose name marks a small fast model (a short list
  in code: haiku, flash, mini, lite, luna, fast, small), else OpenCode's default model **(my call)**. The dropdown lists
  everything OpenCode offers.

**Codex**: `codex exec` with the transcript on stdin, `--ephemeral` (no session files), read-only sandbox, `--json`,
`--skip-git-repo-check`, `--ignore-user-config`, `-m <model>`. Uses the user's own Codex sign-in. (OpenAI's "Sign in with
ChatGPT" would be faster, but CUE would then hold a login token, against decision 1.)

**Grok: not offered — owner's decision 09.11.2026.** A named-pipe route was built and passed its checks (09.11:
the text through a one-shot pipe only CUE's own grok.exe could open, one turn, the session Grok keeps deleted after
each answer), but grok-4.7, the only Grok model that cleans dictations without acting on or commenting on them, takes
6–9 s for a short cleanup, against 3–4 s for Codex. The owner chose OpenCode and Codex only. The route lives in the
history (commits 5999546 and 2054e46, reverted) and the evidence in `app/crates/engines/tests/fixtures/agents/README.md`;
`AgentId::Grok` stays so settings that name it still parse.

### Ask and translation

- **Translation** follows the cleanup route, as today: the agent, or the local model. On Без ИИ or nothing chosen,
  translation is unavailable: the Drop says so ("Перевод: выберите AI") and nothing untranslated is pasted, as today.
- **Ask** follows the same route:
  - Through an agent: the same question, context (selection, before the caret, app, today) and limits as today, in a
    prompt that asks for a JSON answer `{"answer": …, "write": true|false, "replace": true|false, "sources": [{"url",
    "title"}]}` in place of the Anthropic `write_to_field` tool, so the write targets, the card's buttons (Replace over a
    writable selection), follow-ups and source chips stay as they are **(my call)**. A follow-up sends the whole
    conversation again as text (Codex runs with no session of its own). Web search on when `ask.web_search` is on and the agent can search (Codex and Grok can; OpenCode when its model
    has a search tool) — otherwise answered without it, and the card says "без поиска". The answer streams into the card
    when the agent streams events; otherwise it appears whole.
  - Through the local model: answered without web search, the card says so — only by a model that follows instructions
    (RuadaptQwen3 4B, or a found general model). cleanup-1 and S1 Mini only clean: with one of them chosen, Ask and
    translation are unavailable and the Drop says "Спросить: нужна модель, которая отвечает" / "Перевод: нужна модель,
    которая переводит"; their rows say "Только чистка", and the Instructions row says "Эта модель не читает инструкции"
    **(my call)**. The dictionary still reaches their text through the no-AI layer.
  - On Без ИИ or nothing chosen: Ask is unavailable; the Drop says "Спросить: выберите AI".
  - `ask.model` goes; Ask uses the cleanup's agent and model.

### The Drop's words

`claudeKey` and `cleanupKey` go. New words (with "!" in amber, no window): `<Agent>: войдите заново`, `<Agent> не
найден`, `<Agent> не ответил` (only when the fallback is "paste as said" — with the local fallback the text is cleaned and
nothing is shown), `Перевод: выберите AI`, `Спросить: выберите AI`.

### Models on this computer (by characteristics, not measured)

The catalog (`core/llm.rs`) becomes:

| Model | Repo, quant, size | Languages | Why | Tag |
|---|---|---|---|---|
| Spokenly cleanup-1 | `spokenly/cleanup-1-gguf`, Q8_0, 0.64 GB | RU + EN (Russian weaker) | Built for dictation cleanup (fillers, self-corrections); ~0.3 s for a sentence on a 6-core CPU (its card) | Рекомендуем · быстрая |
| RuadaptQwen3 4B Instruct | `RefalMachine/RuadaptQwen3-4B-Instruct-GGUF`, Q4_K_M, 2.49 GB | RU + EN, translates | Qwen3 4B (already works in CUE) with a Russian tokenizer: fewer tokens, so faster on Russian | Рекомендуем · точнее по-русски |
| S1 Mini | as in the model-picks spec, 0.46 GB | EN only | Superwhisper's cleanup model; fastest for English | Рекомендуем for English-only users |

Apache-2.0 for the first two (check S1 Mini's license before shipping). Qwen3 4B Instruct, Gemma 4 E2B and Qwen3 1.7B leave
the catalog (Gemma rejected by the owner, Qwen3 1.7B weak here); a downloaded one stays usable under "Другие". cleanup-1
and S1 Mini need their own input formats (their cards); the JSON grammar path stays for the others. LFM3 1.7B (lists
Russian, 1.1 GB) is a later candidate: it needs a newer llama.cpp than `llama-cpp-2 0.1.161` and its license is free only
under US$10M revenue.

### The no-AI layer

Built as `2026-11-05-no-ai-layer-design.md` says (exact matches only), and run before every route, the agents included.

### First run and the welcome window

- The welcome window's Keys step becomes **"Кто чистит текст"**: the same three-part control and lists as the Cleanup
  page. Its default part: Ваш AI with OpenCode if found, else the first found agent; with none found, Этот компьютер with
  nothing chosen and the recommended models highlighted (nothing downloads until pressed).
- The welcome window no longer opens because a key is missing; it opens on first run as today.

### Settings and migration

- `refine` becomes: `where` (`off` | `local` | `agent`), `local` (model id), `agent` (`opencode` | `codex` | `grok`),
  `models` (per agent), `fallback` (`raw` | `local`), and today's thresholds. `style`, `custom`, `connection`, `runs` go
  (read once for migration, never written). `ask.model` goes.
- Migration on the first start of this version **(my call)**:
  - `runs: local` → `where: local`, same model.
  - `runs: cloud` → `where: agent` with the first agent found (OpenCode, Codex, Grok); none → `where: local`, nothing
    chosen. The Cleanup row then reads "Не выбрано" until the user chooses.
  - `style: custom` with text → that text becomes `instructions.md`; otherwise the standard file.
  - CUE deletes its eight API key entries (service `CUE`: anthropic, openrouter, openai, gemini, deepseek, mistral, groq,
    xai) from Credential Manager. It keeps `habits-secret`, which is not a key.
- `cue-cli keys` goes; the leak scan keeps checking for API-key-looking strings. Developer examples that call a cloud API
  (`refine_compare`, `ask_compare`, `refine_probe`, `ask_probe`, `warm_probe`) read a key from an environment variable,
  never from Credential Manager, or are removed **(my call)**.

### What goes from the code

`connections.rs` (the eight providers), `engines/check.rs`, the key commands (`key_state`, `key_set`, `key_check`),
`keys.rs` except the habit secret, the Anthropic, OpenRouter and Chat request paths for cleanup and Ask (`claude.rs`,
`chat.rs`, `openrouter.rs`, the Anthropic parts of `ask.rs`), the Anthropic warm-ups (`warm`, the HEAD ping), the
Keys UI (`keys.ts`, key rows, the welcome Keys step), prices and USD in usage (`Pricing`, `refine_cost`; usage keeps words
and time). Reusable parts move to the agent path: the system text, the output checks, `classify`-style failure mapping.

### Docs to update

README (Keys, Privacy, Features, API keys, Settings, Costs, the benches, Privacy in detail, Known limitations),
ARCHITECTURE (Refine, Translation, Cache warm-up, Habits, Ask, Settings tray, Welcome window, Settings, Usage accounting,
Logging and privacy rules, Bench), `docs/notes/local-cleanup.md`, `optimize-and-settings.md`, `model-picks.md`, and a new
note for this line of work.

## Left to the builder

- Module names and layout of the agent path (one trait over the three agents is the obvious shape).
- How each agent's CLI reports "not signed in" and its streaming events, checked on the installed versions.
- How the terminal window for "Открыть в <agent>" is opened (Windows Terminal when present, else a console).
- The exact Russian and English strings beyond those quoted here, in the style of `text.ts`.
- Test layout: table tests for migration and the no-AI layer; a fake agent executable for the agent path (no network, no
  real agent in tests).
