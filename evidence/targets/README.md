# evidence/targets

Raw data behind `docs/TARGETS.md`. Collected 2026-10-19.

| File | What it holds |
|---|---|
| `rtx5070_specs.json` | RTX 5070 specs from NVIDIA's RTX Blackwell whitepaper (Table 6) and product page |
| `gguf_tensor_summary.json` | Per GGUF file: total tensor bytes, parameter count, input embedding and output tensor type and bytes. Read from each file's header on Hugging Face (first 24 MB via an HTTP range request) |
| `ceilings.json` | Decode ceilings (672 GB/s / bytes read per token) at 0, 4k, 16k, 32k context; NVFP4 and FP8 ideal ceilings; prefill ceilings per precision |
| `llm_published.json` | Every published LLM speed row collected (RTX 5070 first, RTX 5070 Ti fallbacks flagged `fallback_card: true`), plus the list of runtimes where nothing was found |
| `whisper_published.json` | Speech-to-text speed rows |
| `gaming_published.json` | Measured gaming lever effects (DLSS modes, frame generation, Reflex, ReBAR, OC, undervolt, Game Mode) |
| `sources_ledger.json` | Every cited source with the verbatim quote(s) that were checked against the fetched page text |

Method notes:

- Bytes read per decoded token = sum of all tensor bytes, minus the input embedding table for models with untied embeddings (Qwen3-8B, Qwen3-14B, Llama 3.1 8B). Tied models (Qwen3-4B, Gemma 3 12B) read the full table every token as the output layer.
- KV cache bytes per context token (FP16) = 2 × layers × KV heads × head dim × 2. Gemma 3 12B: 8 global layers read the full context, 40 sliding-window layers read at most 1,024 tokens.
- Bandwidth is the 672 GB/s spec peak. Achievable bandwidth on the real card is not yet measured.
- Re-running the collection: the GGUF header read and the arithmetic were done in a scratch script outside this repo (the repo holds no Python). The numbers can be re-derived from the tensor table that `llama.cpp`'s `gguf-dump` prints for each file.
