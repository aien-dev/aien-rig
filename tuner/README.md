# rig-tune: the AIEN Rig FPS tuner (Windows)

`rig-tune.exe` finds what costs frames on a Windows gaming PC, changes only
safe, reversible Windows settings (asking before each one), measures every
game with its own frame counter, and keeps a before/after receipt per game.
Everything it changes can be put back exactly with `rig-tune undo`.

Built for an RTX 5070 on Windows 11; works on any Windows 10/11 PC.
Rust only (the `windows` crate). No overclocking, no BIOS, no drivers, no
services, nothing injected into games.

## Quick start (open Terminal as administrator)

```
rig-tune scan                      # what is costing frames, with a plain-English fix for each
rig-tune games                     # installed Steam / Epic / Xbox games and the exe to measure
rig-tune measure Cyberpunk2077.exe --seconds 60 --label before
rig-tune apply                     # asks before every change, logs the old value first
(restart if GPU scheduling was turned on, then play the same spot again)
rig-tune measure Cyberpunk2077.exe --seconds 60 --label after
rig-tune compare "Cyberpunk 2077"  # average FPS, 1% low, 0.1% low, before vs after
rig-tune undo                      # put everything back exactly (undo --list shows the log)
rig-tune game-mode on              # pause the AIEN Rig AI server while playing; off to resume
```

## What `scan` checks

| Item | How it is read | What rig-tune does about it |
|---|---|---|
| Graphics card, video memory, driver version | DXGI; NVML (`nvml.dll`, part of the NVIDIA driver, not CUDA) | report |
| Resizable BAR | NVML BAR1 size (over 256 MB means on) | explains the BIOS switch; never touches the BIOS |
| PCIe link width | NVML | flags a card running on fewer lanes than it supports |
| Monitor refresh rate vs. the highest it supports | display modes from the driver | `refresh-rate-max` (reverts unless you confirm within 15 s) |
| Monitor plugged into the motherboard, not the card | which adapter drives each screen | tells you to move the cable |
| Hardware-accelerated GPU scheduling | registry `HwSchMode` | `hags-on` (counts after a restart) |
| Windows Game Mode | registry `AutoGameModeEnabled` | `game-mode-on` |
| Xbox Game Bar background recording | registry `HistoricalCaptureEnabled` | `background-recording-off` |
| Optimizations for windowed games | registry `DirectXUserGlobalSettings` | `windowed-optimizations-on` (other keys in that value are kept) |
| Power plan | Windows power API | `power-plan-high` (only to plans this PC offers) |
| G-SYNC / VRR | NVIDIA keeps it private | step-by-step instructions |
| Background CPU and video-memory hogs | process CPU times, `GPU Process Memory` counters | tells you what to close |
| AIEN Rig AI server running | process list | suggests `game-mode on` |

## How frames are counted

`measure` starts a real-time ETW session (Event Tracing for Windows, built
into Windows) and listens to present events the graphics stack already emits:

- `Microsoft-Windows-DXGI` event 42 (Present_Start): Direct3D 10/11/12 games,
  and Vulkan/OpenGL games the NVIDIA driver presents through DXGI.
- `Microsoft-Windows-D3D9` event 1: older Direct3D 9 games.
- `Microsoft-Windows-DxgKrnl` present events: any API. Used only when the two
  above saw nothing for the game, counting one event type per process so no
  frame is counted twice.

Nothing is injected into the game. Windows requires administrator rights for
ETW sessions.

The numbers (the same definitions are written into every receipt):

- average FPS = frames / total time of those frames (not the mean of per-frame FPS)
- 1% low = 1000 / mean of the slowest 1% of frame times (at least one frame); 0.1% low the same with 0.1%
- p99 / p99.9 FPS = 1000 / the 99th / 99.9th percentile frame time, for tools that call that the "1% low"

Receipts are JSON files in `%LOCALAPPDATA%\AIEN-Rig\receipts`, one per run,
named `<game>-<UTC time>-<label>.json`. Each holds every frame time (so the
numbers can be recomputed), a 600-point frame-time graph series, the frame
source, the GPU, driver, display mode and power plan at the time, and which
rig-tune changes were active.

## Safety rules for changes

- Only the six tweaks above exist. Each is one Windows setting.
- The old value is written to `%LOCALAPPDATA%\AIEN-Rig\undo-log.json` before
  the change is made, so even a power cut mid-change can be undone. The change
  is read back; if it did not stick, the old value is put back.
- `rig-tune undo` restores newest first. If you or another program changed a
  setting after rig-tune did, undo leaves it alone and says so (`--force`
  overrides). Values that did not exist before are deleted again.
- Display modes are only set to modes the driver lists for that monitor, are
  tested with the driver first, and revert unless you confirm you can see the
  screen.

## Game mode contract (for the AIEN Rig app and AI server)

`rig-tune game-mode on` writes `%LOCALAPPDATA%\AIEN-Rig\game-mode.json` with
`"active": true` (a server can watch it, finish its request and free video
memory), then suspends every thread of processes named
`aien-rig-server.exe`, `aien-rig-engine.exe`, `aien-engine.exe` or
`aien-serve-http.exe` (or `--process NAME`), so they use no CPU or GPU time.
Their video memory stays reserved while suspended. `game-mode off` resumes
exactly those processes after checking each PID still belongs to the same
program.

## Building and testing

```
cd tuner
cargo test                                     # pure logic tests, any OS
cargo build --release                          # on Windows: target\release\rig-tune.exe
cargo check --target x86_64-pc-windows-msvc    # from macOS/Linux
```

The crate is its own Cargo workspace root, so it builds the same whether or
not the repo has a top-level workspace (a top-level workspace should list
`tuner` under `exclude`).

CI (`.github/workflows/tuner.yml`, windows-latest) builds the release exe,
runs the unit tests and Windows OS tests (registry, power API), runs clippy
with warnings as errors, then `ci/windows-e2e.ps1`:

1. `scan` must run and return findings.
2. A small Direct3D 11 program (Microsoft's WARP software renderer, no GPU
   needed) presents at 120 fps; `rig-tune measure` must agree with the
   program's own count within 3%. First green run (Actions run 39292662908):
   probe presented 120.00 fps, rig-tune measured 120.00 fps average,
   1% low 117.6, 1808 frames from DXGI event 42.
3. Settings are put in a "needs tuning" state, `apply --yes` must change each
   one, and `undo` must restore every value exactly (including deleting
   values that did not exist).
4. A stand-in `aien-rig-server.exe` is started; after `game-mode on` its CPU
   time must stop moving, after `game-mode off` it must move again.

The exe and the CI receipts are uploaded as build artifacts.

## Not yet verified

- Not yet run on the RTX 5070 PC. NVML reads (Resizable BAR, PCIe link) and a
  real monitor refresh-rate change have only been compile-checked; CI has no
  NVIDIA card and only a virtual display.
- The DxgKrnl fallback (Vulkan/OpenGL games that do not go through DXGI) is
  unit-tested on synthetic events only.
- No game FPS gains are claimed yet. Gains get claimed only from
  before/after receipts on the real PC.
