# rig-listen on GB10: WER and speed, 2026-10-19

Model: whisper-large-v3-turbo (sources and sha256 in `models/`). Audio: three LibriSpeech
test-clean utterance groups, 4.6 to 8.2 minutes each, joined into one file (checksums in
`fixtures/`; the 1089 file is an mp4 container, the other two flac, so video input is covered).
Word error rate is `rig-listen wer` (lower case, punctuation removed) against the LibriSpeech reference text.

## Accuracy (identical on CPU and on Omega: same words out)

| Fixture | Audio | Reference words | Substitutions | Deletions | Insertions | WER |
|---|---|---|---|---|---|---|
| 1089-134686 (mp4) | 276 s | 721 | 6 | 0 | 0 | 0.83 % |
| 1188-133604 (flac) | 492 s | 1296 | 11 | 0 | 1 | 0.93 % |
| 2961-960 (flac) | 282 s | 640 | 6 | 3 | 1 | 1.56 % |

## Speed (seconds to transcribe, model load not counted; 1x = real time)

| Run | Fixture | Transcribe s | Encoder s | Decoder s | x real time | Box load at end |
|---|---|---|---|---|---|---|
| `cpu/` (our Rust kernels) | 1089 | 353.1 | 321.7 | 30.9 | 0.78 | 70.5 |
| `cpu/` | 1188 | 724.7 | 632.0 | 91.8 | 0.68 | 87.4 |
| `cpu/` | 2961 | 325.5 | 282.4 | 42.3 | 0.87 | 56.7 |
| `omega-decoder-cpu/` (Omega GPU encoder, decoder on CPU) | 1089 | 104.7 | 74.9 | 29.8 | 2.61 | 85.6 |
| `omega-decoder-cpu/` | 1188 | 188.1 | 127.9 | 60.0 | 2.61 | 64.9 |
| `omega-decoder-cpu/` | 2961 | 105.1 | 76.9 | 28.1 | 2.68 | 56.7 |
| `omega-decoder-gpu-v3/` (Omega GPU for encoder and decoder matrices) | 1089 | 95.8 | 76.4 | 19.3 | 2.87 | 73.7 |
| `omega-decoder-gpu-v3/` | 1188 | 154.6 | 123.0 | 31.4 | 3.17 | 71.4 |

Read these as slow-side numbers: the box was shared (load average 55 to 90 on 20 cores) during every run, and
the encoder still does layer norm, GELU, softmax and the convolutions on the CPU; only the matrix
multiplies and attention run on the Omega GPU path. Selftest: Omega matmul agrees with the CPU
to 7e-6 absolute (`omega-first-chip-run/`, `omega-decoder-cpu/gpu-selftest.json`); encoder-shaped
matmuls run at 0.8 to 1.1 TFLOP/s against 60 to 116 GFLOP/s on the CPU.

## Caveats, stated plainly

- `omega-decoder-gpu-v3/` receipts say `git_sha 19af927e9dd2`. That is wrong: the build script did not
  re-read HEAD, so a binary built from the later source (decoder on GPU, commit "decoder matrices on the GPU
  by default", now `0c063a7`) carried the old label. Fixed in `322d1eb` (receipts now say `-dirty` for
  uncommitted builds and record the Omega commit). The v3 run was not repeated (below).
- Only 2 of the 3 fixtures have a decoder-on-GPU run. The third (2961) failed with
  `CHIP_FAIL: m16_native_create_channel: RM_ALLOC class 0xcec0 status 0x51` (`omega-decoder-gpu-v3/gpu3.log`).
  A clean rerun with the fixed build was made at 14:27 CDT and failed the same way, even for the
  selftest (`chip-unavailable/gpu4.log`). At that moment three `aien-serve-http` Qwen servers held
  about 93 GB of the GB10 memory and, presumably, its channels. We did not stop them (not ours);
  the cause is a guess from timing, not a measurement. The rerun is open work.
- History note: the earlier commits were rewritten once (build directories had been committed by mistake);
  old to new commit ids: 19af927 (same), cf1f52a to 1c7029c, 1523aae to 47dd136, 5108080 to 0295e71,
  ac55d6e to 0c063a7, 55fea2b to 322d1eb. Receipts in `cpu/` and `omega-decoder-cpu/` name unchanged commits.
- `creator-check/captions.clean.srt` shows `rig-create --captions-only` reading the speech JSON
  (schema `aien-rig-transcript/1`) and writing clean captions.
- Builds and runs went through `quiet-build` / `quiet-measure` with `/tmp/aien-gb10.lock`.
