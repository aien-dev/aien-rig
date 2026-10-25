# AIEN Rig (local AI and a measured FPS tuner for RTX gaming PCs)

> Pre-alpha, under active build. Local AI and a measured FPS tuner for RTX gaming PCs, starting with the RTX 5070 (12 GB). Five parts: an own C++/CUDA inference engine, speech-to-text, a YouTube creator helper, a Windows FPS tuner, and one Windows app that ties them together. The component directories currently hold only placeholder READMEs; there is no build system or CI yet. Nothing has been tested on an RTX 5070.

Shared rules: read https://www.aienos.com/agents/RULES.md first. This file adds what is specific to this repository; where the two differ, this file wins here.

## Scope

- Applies to the whole repository.
- `engine/`: C++/CUDA inference engine for RTX cards (Qwen3 dense, INT4/FP8, CUDA graphs, OpenAI-compatible HTTP server). Goal: faster than llama.cpp on the same card, measured.
- `speech/`: own speech-to-text (Whisper-family runtime) for video transcripts.
- `creator/`: YouTube helper: titles, descriptions, chapters, Shorts picks, captions.
- `tuner/`: Windows FPS tuner: measures each game, applies only safe changes, keeps receipts, can undo everything.
- `app/`: the Windows app, packaging, Windows CI and release (`app/`, `.github/`).
- `docs/`: plan, targets (`docs/TARGETS.md` once written); `evidence/`: receipts.

## Installation

```bash
git clone https://github.com/aien-dev/aien-rig.git
cd aien-rig
```

No build exists yet. When it does, engine and speech work needs an NVIDIA RTX card (target: RTX 5070, 12 GB), and tuner/app work needs Windows.

## Usage

Work by stream. `docs/PLAN.md` maps each stream to its directory and branch:

| Stream | Directory | Branch |
|---|---|---|
| Engine core | `engine/` | `engine/core` |
| Targets and yardsticks (ceiling math, llama.cpp and others on 5070) | `docs/TARGETS.md`, `evidence/` | `docs/targets` |
| FPS tuner (Windows) | `tuner/` | `tuner/v0` |
| Speech-to-text | `speech/` | `speech/v0` |
| YouTube helper | `creator/` | `creator/v0` |
| App, packaging, Windows CI, release | `app/`, `.github/` | `app/v0` |

## Conventions

- Owners work on their own branch and directory. `main` is merged by the desk only.
- Every speed claim links to a receipt in `evidence/` and names the card it ran on.
- Compare against llama.cpp on the same card before claiming "faster".
- License is AGPL-3.0; keep new files compatible.

## Safety and approvals

- The tuner changes settings on users' machines: it must apply only safe changes, record a receipt for each, and be able to undo every change. Do not add a change the tuner cannot revert.
- Do not publish a performance number without a receipt from real hardware.
- Never commit tokens, credentials, or private data. Work on a branch and merge through a pull request.

## Further reading

- `README.md`: overview and status.
- `docs/PLAN.md`: streams, directories and branches.
