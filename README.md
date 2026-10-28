# AIEN Rig

Local AI and a measured FPS tuner for RTX gaming PCs, starting with the RTX 5070 (12 GB).

- **engine/**: our own C++/CUDA inference engine for RTX cards. Goal: faster than llama.cpp on the same card, measured.
- **speech/**: our own speech-to-text for video transcripts.
- **creator/**: YouTube helper: titles, descriptions, chapters, Shorts picks, captions.
- **tuner/**: Windows FPS tuner: measures each game, applies only safe changes, keeps receipts, can undo everything.
- **app/**: one Windows app that ties it together.

Status: pre-alpha, under active build. Not tested on an RTX 5070 yet. Every speed claim links to a receipt in evidence/.

## Human-Directed Agentic Engineering

Development follows Human-Directed Agentic Engineering (HDAE): humans define goals,
architecture, constraints, acceptance criteria, and stopping points; agents implement,
execute checks, diagnose failures, and revise within those boundaries. Human review
retains responsibility for claims, acceptance, and release decisions. This method does
not imply that planned components or hardware performance have been verified.

For the documented method and a systems-software case study, see Gregory M. Kurtzer,
[*Ginux: Building an Independent Kernel That Runs Linux Userspace through
Human-Directed Agentic Engineering* (2026)](https://doi.org/10.5281/zenodo.23682548).


License: AGPL-3.0.
