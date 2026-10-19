# speech: rig-listen

Speech to text for AIEN Rig. Turns the audio of a video into a transcript with
timestamps, on the user's own machine, with our own runtime (Rust, plus Omega on the GPU).
No C++, no CUDA, no PyTorch, no whisper.cpp.

Model: OpenAI Whisper large-v3-turbo (809 M parameters, 32 encoder + 4 decoder layers,
MIT licence), weights as safetensors from Hugging Face. At 16-bit it is about 1.6 GB, so it
fits a 12 GB RTX 5070 with plenty of room. Any Whisper checkpoint in the Hugging Face
layout works (tested: whisper-tiny and whisper-large-v3-turbo).

## Use

```
rig-listen my-video.mp4                      # writes my-video.mp4.transcript.json
rig-listen --language en --out t.json my-video.mov
rig-listen wer reference.txt t.json          # word error rate against a known text
```

Options: `--model DIR`, `--out FILE`, `--language CODE|auto`, `--backend cpu|omega`,
`--threads N`, `--receipt FILE` (speed receipt), `--machine LABEL`, `-q`.

## Output

```json
{"segments": [{"start": 0.0, "end": 5.68, "text": "..."}], "language": "en"}
```

Times are seconds from the start of the file. This is the fallback shape from the task:
`creator/TRANSCRIPT.md` did not exist on `creator/v0` when this was written
(checked 2026-10-19). If the creator stream defines a different shape, `to_json` in
`src/transcribe.rs` is the one place to change.

## What is inside

| Step | File | Notes |
|---|---|---|
| Open audio or video | `src/audio.rs` | symphonia (pure Rust): mp4/m4a/mov/mkv/webm/wav/flac/mp3/ogg; AAC, ALAC, FLAC, MP3, Vorbis, PCM. First audio track; video tracks skipped. Opus is not supported yet. |
| Mono, 16 kHz | `src/audio.rs` | downmix, polyphase windowed-sinc resampler (Blackman-Harris, 32 zero crossings) |
| Log-mel features | `src/mel.rs` | Whisper's recipe: 400-point STFT, hop 160, Slaney mel filters (80 or 128), log10, max-8 floor |
| Weights | `src/weights.rs` | safetensors reader (mmap), byte-level BPE decoding from tokenizer.json |
| Encoder + decoder | `src/model.rs` | Whisper architecture; decoder with a key/value cache |
| Matrix multiply, CPU | `src/gemm.rs` | our own packed, register-blocked kernel, threads with rayon |
| Matrix multiply, GPU | `src/omega.rs` | Omega `libomega_gpu.a` resident bf16 matmul (our own Blackwell code generator, no CUDA) |
| Long audio | `src/transcribe.rs` | 30 s windows, greedy decoding with Whisper's timestamp rules, segments at timestamp pairs, no-speech skip |
| Word error rate | `src/wer.rs` | Levenshtein over normalised words |

## Build

CPU only (works anywhere Rust does, including Windows):

```
cargo build --release
```

With the Omega GPU path (GB10 today):

```
make -C <omega checkout> libomega_gpu PHYSICS_DIR=<physics at physics.lock>
OMEGA_GPU_LIB=<omega checkout>/build/libomega_gpu.a cargo build --release --features omega
```

## Models

`scripts/fetch-model.sh <repo> <revision> <name> <licence>` downloads into `~/models/<name>`,
checks every size and every LFS sha256 against the Hugging Face listing, and writes
`<name>.SOURCE` and `<name>.sha256`. Pinned so far:

| Name | Repo @ revision | model.safetensors sha256 |
|---|---|---|
| whisper-large-v3-turbo-41f01f3f | openai/whisper-large-v3-turbo @ 41f01f3fe87f28c78e2fbf8b568835947dd65ed9 | 542566a422ae4f3fd23f1ba11add198fca01bbf82e66e6a2857b3f608b1eb9d1 |
| whisper-tiny-169d4a43 | openai/whisper-tiny @ 169d4a4341b33bc18d8881c4b69c2e104e1cc0af | see ~/models/whisper-tiny-169d4a43.sha256 |

## Limits (today)

- Greedy decoding at temperature 0 only: no beam search and no temperature fallback, so a
  rare repetition loop is not retried.
- No word-level timestamps (segment level only).
- The RTX 5070 GPU path is not built yet: Omega's kernels target the GB10 (sm_121). On the
  5070 the CPU path runs today.
- Opus audio (common in .webm downloads) is not decoded yet.
