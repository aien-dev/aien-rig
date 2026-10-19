# AIEN Rig

Local AI and a measured FPS tuner for RTX gaming PCs, starting with the RTX 5070 (12 GB).

- **engine/**: our own C++/CUDA inference engine for RTX cards. Goal: faster than llama.cpp on the same card, measured.
- **speech/**: our own speech-to-text for video transcripts.
- **creator/**: YouTube helper: titles, descriptions, chapters, Shorts picks, captions.
- **tuner/**: Windows FPS tuner: measures each game, applies only safe changes, keeps receipts, can undo everything.
- **app/**: one Windows app that ties it together.

Status: pre-alpha, under active build. Not tested on an RTX 5070 yet. Every speed claim links to a receipt in evidence/.

License: AGPL-3.0.

## Download (Windows)

https://github.com/aien-dev/aien-rig/releases/latest/download/aien-rig-windows-x64.zip

Extract it and read `README-FIRST.txt`. Built and tested by CI on Windows; not yet run on an RTX 5070.
