# app: aien-rig.exe

One Windows program. It serves a local page on `http://127.0.0.1:18100`
(next free port up to 18109) and opens it in the browser.

- **Play**: runs `rig-tune.exe` scan / measure / apply / undo, shows the result
  and a receipt for every run; watches for games and, while one runs, pauses
  the engine and makes sure Windows Game Mode is on (restored afterwards).
- **Create**: Windows file picker for a caption file or a video; videos go
  through `rig-listen.exe` (speech to text) first, then `rig-create.exe`.
  Shows titles, description with chapters, tags and Shorts picks from its
  `result.json`, with Copy buttons.
- **Chat**: starts and stops the engine inside WSL2 and talks to it over
  `127.0.0.1:18120` (OpenAI-style API), the address rig-create also uses.
- **Settings**: games to watch, engine command, WSL distribution, address.

How it calls the other programs: [../docs/APP-CONTRACT.md](../docs/APP-CONTRACT.md).
Why the engine runs in WSL2: [../docs/WINDOWS-GPU-PATH.md](../docs/WINDOWS-GPU-PATH.md).

Data folder: `%LOCALAPPDATA%\AIEN-Rig` (`config.json`, `receipts\`,
`create\`), shared with rig-tune; `AIEN_RIG_HOME` overrides it.

Safety: listens on 127.0.0.1 only, answers only to `Host: 127.0.0.1:<port>` or
`localhost:<port>`, and requires an `X-AIEN-Rig: 1` header on every POST so
other web sites open in the same browser cannot drive it.

Build and test (any OS; Windows-only parts are behind `cfg(windows)`):

```
cargo test -p aien-rig
cargo check -p aien-rig --target x86_64-pc-windows-msvc
```

Dependencies: serde, serde_json, ctrlc. The HTTP server and client are in
`src/http.rs` (std only).
