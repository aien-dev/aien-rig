# Plan (2026-10-19)
Owners work on their own branch and directory. Main is merged by the desk only.
| Stream | Dir | Branch |
|---|---|---|
| Engine core (Qwen3 dense, INT4/FP8, CUDA graphs, OpenAI HTTP server) | engine/ | engine/core |
| Targets and yardsticks (ceiling math, llama.cpp and others on 5070) | docs/TARGETS.md, evidence/ | docs/targets |
| FPS tuner (Windows) | tuner/ | tuner/v0 |
| Speech-to-text (our own Whisper-family runtime) | speech/ | speech/v0 |
| YouTube helper | creator/ | creator/v0 |
| App, packaging, Windows CI, release | app/, .github/ | app/v0 |
