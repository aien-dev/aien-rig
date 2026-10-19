# engine

The model-serving side of AIEN Rig: our own inference engine ("omega") behind `aien-serve-http`, serving one
model inside a 12 GiB memory budget (the RTX 5070 has 12 GB).

**Honest scope.** Everything below was measured on the DGX Spark (GB10), not on an RTX 5070. The same code runs
the same model and the same budget check, but speed on an RTX 5070 is NOT MEASURED. The ceiling per model and
format for the 5070 (672 GB/s) is on paper in [CEILING.md](CEILING.md), every figure with an aien-calc receipt.

## What serves today

`aien-serve-http --engine omega --model-dir <Qwen3-8B FP8 checkpoint> --weights w4 --mem-budget-gib 12 --max-context 32768`
(source: `crates/aien-serve-http` and `crates/aien-inference-abi/src/qwen3_dense_*` in the `rig/engine-12gb` branch of
aien-sovereign-core).

| format | on the card | speed through HTTP (greedy) | answers vs host reference |
|---|---|---|---|
| Qwen3-8B FP8 (Qwen's own file) | 11.6 GiB, context shrunk to 28,928 tokens | 21 to 23 tok/s | 3 of 3 identical, control fails as it must |
| Qwen3-8B MXFP4 (4 bit, made from the FP8 file) | 9.1 GiB, full 32,768 tokens | 28 to 29 tok/s (first attempt, see below) | see the http-w4-12gib line below |

Chip-level runs (no HTTP): FP8 15.63 tok/s, MXFP4 20.17 tok/s (5.158 GiB of weights on the card), both with 5 of 5
tokens identical to the host reference and a swapped-KV control that fails as it must.

**Best model for 12 GiB: Qwen3-8B in MXFP4.** It is the only format that fits with the full 32k context, it is
about 1.3x faster than FP8 here, and the room left over is KV cache. The bf16 8B file (14.1 GiB) is refused by
the budget check, and FP8 with a 32k context is refused by policy (12.52 GiB needed); both refusals are logged.

**Cost of 4 bit.** MXFP4 changes the weights: relative RMS error of the weights is 0.1176 against the FP8 file.
Replies are checked against a host reference that uses the same 4-bit bytes, so the gate proves the engine does
what the numbers say; it does not prove the 4-bit answers match the FP8 answers. A quality comparison against FP8
is not done here.

## How it is checked

- `tools/rig/http-gate.sh`: starts our server on a spare port, sends greedy chat requests, reads the exact prompt
  and reply token ids from the server, recomputes the same prompts on the host reference, and requires identical
  ids. A control (one reference id changed) must report MISMATCH, so the gate cannot pass without checking.
- `tools/rig/chip-run.sh`: runs a chip job with memory and lock bookkeeping and writes a receipt.
- Evidence, including every failed attempt kept as it happened:
  `docs/inference/evidence/rig-engine-20261019/` in aien-sovereign-core (branch `rig/engine-12gb`).

## Distance to the ceiling

On the GB10 (273 GB/s) the FP8 GEMV reaches about 77 percent of nominal bandwidth. End to end the served speed is
well below that: the 8B FP8 decode is 22 tok/s against a GB10 paper ceiling of 33.3 tok/s (calc:1eb080c8; 8.19 GB per token
at 273 GB/s). Closing that gap is the next engine work. On the RTX 5070 the paper ceiling for the MXFP4 8B is 136
tok/s at short context (CEILING.md); no 5070 run exists yet.

## Known limits

- Other jobs on the same GPU hold the chip lock for minutes; the server answers 503 "GPU busy" after 300 s and the
  gate retries. Speeds above include no lock wait (taken from server-side decode time).
- Loading can fail with NV_ERR_NO_MEMORY when the box is short of free memory; the gate waits for 16 GiB free.
- The embedding table (1.16 GiB) stays in system RAM in the 4-bit configuration.
