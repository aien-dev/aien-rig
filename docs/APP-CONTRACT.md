# How aien-rig.exe calls the other programs

`aien-rig.exe` (crate `app/`) is the page. The work is done by sibling
programs that sit in the same folder. This is the contract the app assumes.
Every argument list below is a default in the app's settings
(`%LOCALAPPDATA%\AIEN Rig\config.json`), so a different CLI only needs a
settings change, not a new app build.

## Common rules

- The app runs the program with no console window, captures stdout and
  stderr, and stops it after `tool_timeout_secs` (default 1800 s).
- If stdout is a single JSON object or array, the page shows it as structured
  results. Otherwise it shows the raw text. Exit code 0 means success.
- The app saves a receipt for every run in `%LOCALAPPDATA%\AIEN Rig\receipts\`
  (`<UTC stamp>-<tool>-<action>.json`: program, args, exit code, seconds,
  stdout, stderr, and stdout parsed as JSON). The tuner keeps its own receipts
  too; the app's receipt is the record of what the app asked for.
- Only one tool runs at a time.

## rig-tune.exe (tuner stream, `tuner/`)

| Button | Default arguments | JSON the page understands |
|---|---|---|
| Scan | `scan --json` | `{"games":[{"id":"cs2","name":"Counter-Strike 2","exe":"cs2.exe"}]}` or a plain list of strings. `id` is what Measure/Apply receive |
| Measure | `measure {game} --json` | any object; shown as is |
| Apply | `apply {game} --json` | any object; shown as is |
| Undo everything | `undo --json` | any object; shown as is |

## rig-create.exe (creator stream, `creator/`)

Default arguments: `{file} --json`, where `{file}` is a full Windows path to a
video (`.mp4 .mov .mkv .webm .avi`) or captions (`.srt .vtt .txt`).

JSON the page understands (every field optional):

```json
{
  "titles": ["...", "..."],
  "description": "...",
  "chapters": [{"time": "0:00", "title": "Intro"}],
  "shorts": [{"start": "1:23", "end": "1:51", "title": "...", "why": "..."}]
}
```

## The engine (engine stream)

- Default mode `wsl`: the app runs
  `wsl.exe [-d <distro>] -- bash -lc 'mkdir -p ~/.aien-rig && echo $$ > ~/.aien-rig/engine.pid && exec <engine_command> >> ~/.aien-rig/engine.log 2>&1'`
  with `engine_command` defaulting to
  `aien-serve-http --engine omega --bind 127.0.0.1 --port 18006`
  (so `aien-serve-http` must be on PATH in a login shell inside WSL).
- Stop: `kill $(cat ~/.aien-rig/engine.pid)` inside WSL. The app never stops
  any other process.
- Health: `GET http://127.0.0.1:18006/v1/models`; the first `data[].id` is used
  as the `model` in chat requests.
- Chat: `POST /v1/chat/completions`, `stream: false`, `max_tokens` up to 4096,
  retried on 503 (honouring `Retry-After`) for up to 60 s.
- While a listed game runs, the app stops the engine and starts it again when
  the game exits (only if it was running before).

Why WSL: docs/WINDOWS-GPU-PATH.md.
