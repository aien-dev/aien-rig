# AIEN Rig

## How it works and how to reproduce

I trace one actual source or design boundary in [the worked lesson](docs/TEACHING.md), with a pinned fresh-checkout exercise and [raw replay receipt](docs/teaching-replay-2026-10-22.txt). The lesson states what remains unimplemented or unverified at this revision.

Proposed local AI and measured FPS tuner for RTX gaming PCs, starting with the RTX 5070 (12 GB). On this branch the five component directories contain plans only.

- **engine/**: proposed C++/CUDA inference engine for RTX cards. Goal: faster than llama.cpp on the same card, measured.
- **speech/**: proposed speech-to-text for video transcripts.
- **creator/**: YouTube helper: titles, descriptions, chapters, Shorts picks, captions.
- **tuner/**: Windows FPS tuner: measures each game, applies only safe changes, keeps receipts, can undo everything.
- **app/**: one Windows app that ties it together.

Status: design/pre-alpha on this branch. No runnable component or RTX 5070 receipt is present here.

License: AGPL-3.0.
