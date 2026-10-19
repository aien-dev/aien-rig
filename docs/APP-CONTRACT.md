# How aien-rig.exe calls the other programs

`aien-rig.exe` (crate `app/`) is the page. The work is done by sibling
programs in the same folder. This is the contract the app assumes, checked
against the tuner/v0, creator/v0 and speech/v0 branches on 2026-10-19. Every
argument list below is a default in the app's settings
(`%LOCALAPPDATA%\AIEN-Rig\config.json`), so a CLI change needs a settings
change, not a new app build.

## Ports

| What | Address |
|---|---|
| The app's page | `http://127.0.0.1:18100` (next free up to 18109) |
| The engine (aien-serve-http, OpenAI-style) | `http://127.0.0.1:18120` (rig-create's default `--server` is `http://127.0.0.1:18120/v1`) |

## Common rules

- The app runs each program with no console window, captures stdout and
  stderr, and stops it after `tool_timeout_secs` (default 1800 s).
- If stdout is one JSON object or array, the page shows it as structured
  results; otherwise raw text. Exit code 0 means success.
- Every run gets a receipt in `%LOCALAPPDATA%\AIEN-Rig\receipts\`
  (`<UTC stamp>-<tool>-<action>.json`: program, args, exit code, seconds,
  stdout, stderr, stdout parsed as JSON). That folder is shared with
  rig-tune's own receipts (`tuner/src/paths.rs`), so the brother zips one
  folder. The Play tab lists all of them.
- Only one tool runs at a time.

## rig-tune.exe (`tuner/`, standalone crate)

Its CLI is not written yet on tuner/v0 (`src/main.rs` is empty). The app
assumes:

| Button | Default arguments | JSON the page understands |
|---|---|---|
| Scan | `scan --json` | `{"games":[{"id":"cs2","name":"Counter-Strike 2","exe":"cs2.exe"}]}` or a list of strings. `id` is passed to Measure/Apply |
| Measure | `measure {game} --json` | any object |
| Apply | `apply {game} --json` | any object |
| Undo everything | `undo --json` | any object |
| (game starts / ends) | `game-mode on` / `game-mode off` | ignored; logged. This is the contract written in `tuner/src/gamemode.rs` |

## rig-create.exe (`creator/`, standalone crate)

`{file} --server {server} --out {out}`: `{file}` is the caption file (or the
transcript rig-listen wrote), `{server}` is the engine address plus `/v1`,
`{out}` is `%LOCALAPPDATA%\AIEN-Rig\create\<stamp>-<name>\`. The page reads
`<out>\result.json` and shows `titles`, `full_description`, `chapters`
(`time`, `title`), `tags`, `shorts` (`start_time`, `end_time`, `hook`, `why`),
`chapter_problems` and `warnings`.

## rig-listen.exe (`speech/`, standalone crate, optional in the zip)

For video or audio files only: `{file} --out {out}` with `{out}` =
`<results folder>\transcript.json`, then rig-create runs on that file. If
rig-listen.exe is missing, the page asks for a caption file instead.

## The engine (engine stream)

- Mode `wsl` (default): the app runs
  `wsl.exe [-d <distro>] -- bash -lc 'mkdir -p ~/.aien-rig && echo $$ > ~/.aien-rig/engine.pid && exec <engine_command> >> ~/.aien-rig/engine.log 2>&1'`
  with `engine_command` = `aien-serve-http --engine omega --bind 127.0.0.1 --port 18120`
  (`aien-serve-http` must be on PATH in a login shell inside WSL).
- Stop: `kill $(cat ~/.aien-rig/engine.pid)` inside WSL. The app never stops
  any other process.
- Health: `GET /v1/models`; the first `data[].id` is the `model` in chat.
- Chat: `POST /v1/chat/completions`, `stream: false`, `max_tokens` at most
  4096, retried on 503 (honouring `Retry-After`) for up to 60 s.
- While a listed game runs the app stops the engine, runs
  `rig-tune game-mode on`, and makes sure Windows Game Mode
  (`HKCU\Software\Microsoft\GameBar\AutoGameModeEnabled`) is not 0. When the
  game exits it undoes each of those, and starts the engine again only if it
  was running before.

## Workspace layout

Root `Cargo.toml`: `members = ["app"]`, `exclude = ["tuner", "creator", "speech"]`.
The brief asked for all four as members, but tuner and speech declare their
own `[workspace]` (a member cannot) and creator's CI runs `cargo test --locked`
against its own `Cargo.lock`. Excluding them keeps every branch building alone
and merged. `tools/crates.py` lists what is present; the release workflow
builds each with `--manifest-path`.

Why WSL: [WINDOWS-GPU-PATH.md](WINDOWS-GPU-PATH.md).
