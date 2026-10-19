# AIEN Rig targets: RTX 5070 (12 GB)

What everyone else gets on an RTX 5070 for local AI and for gaming, the physical ceilings, and the exact numbers we have to beat. Collected 2026-10-19. Every number has a source link (numbered list at the bottom) or arithmetic shown next to it. Raw data is in `evidence/targets/*.json`.

## 0. The numbers to beat (summary)

Single user, one request at a time, model fully in VRAM, RTX 5070 desktop card.

| Model | THE TARGET decode, 4-bit (tok/s) | Held by | THE TARGET prefill, 4-bit (tok/s) | Physical decode ceiling, 4-bit (tok/s) | Target as % of ceiling |
|---|---|---|---|---|---|
| Qwen3-4B | **180.21** | llama.cpp, Q4_K_M, GPU Battle [8] | none published | 269.7 (Q4_K_M) | 66.8% |
| Qwen3-8B | **109.63** | llama.cpp, Q4_K_M, GPU Battle [8] | **3,487.7** at 4k context (llama.cpp, Hardware Corner [3]) | 143.8 (Q4_K_M) | 76.2% |
| Qwen3-14B | **60.87** | llama.cpp, Q4_K_M, GPU Battle [8] | **2,144.2** at 4k context (llama.cpp, Hardware Corner [3]) | 78.5 (Q4_K_M) | 77.5% |
| Llama 3.1 8B | **119.87** | llama.cpp, Q4_K_M, GPU Battle [8] | **2,927** (LocalScore [6]) | 145.5 (Q4_K_M) | 82.4% |
| Gemma 3 12B | **none published** (we set it ourselves, see 3.4) | n/a | none published | 92.1 (Q4_K_M) | n/a |

Long-context decode targets (llama.cpp, Hardware Corner [3]): Qwen3-8B 59.1 tok/s at 16k and 43.6 tok/s at 32k; Qwen3-14B 40.6 tok/s at 16k.

8-bit: no published RTX 5070 measurement was found for any of the five models, so the 8-bit target is "beat a same-machine llama.cpp Q8_0 run" and the ceiling in section 2.

Speech to text: **43.1x real time** for Whisper large-v3 fp16 on an RTX 5070 with faster-whisper's batched pipeline (141.4 minutes of audio in 196.7 s) [15].

Gaming: there is no single FPS target. Section 5 lists each lever with the measured gain on a 5070 (or the nearest card) and whether an automated tool may touch it.

How to read "Target as % of ceiling": target divided by ceiling, for example 109.63 / 143.8 = 0.762. The gap to 100% is the room an engine has to win on decode.

## 1. RTX 5070 hardware (NVIDIA sources)

From NVIDIA's RTX Blackwell whitepaper, Table 6 (GB205 / RTX 5070) [2] and the RTX 5070 product page [1]:

| Item | RTX 5070 |
|---|---|
| GPU | GB205, Blackwell, TSMC 4N, 31.1 billion transistors, 263 mm² |
| GPCs / TPCs / SMs | 5 / 24 / 48 (full GB205 die has 50 SMs) |
| CUDA cores | 6,144 (128 per SM) |
| Tensor cores | 192, 5th generation (4 per SM) |
| RT cores | 48, 4th generation |
| Base / boost clock | 2.33 GHz / 2.51 GHz [1] (whitepaper boost: 2,512 MHz [2]) |
| Memory | 12 GB GDDR7, 192-bit, 28 Gbps |
| Memory bandwidth | **672 GB/s** (192 bits × 28 Gbps / 8 = 672 GB/s) |
| L2 cache | 49,152 KB (48 MB) |
| L1 / shared memory total | 6,144 KB |
| Register file | 12,288 KB |
| Total graphics power | 250 W; NVIDIA asks for a 650 W power supply [1] |
| PCIe | Gen 5 |
| Video | 1× NVENC (9th gen), 1× NVDEC (6th gen) |

Peak math rates at boost clock, dense (the whitepaper's second number in each pair uses 2:4 sparsity, which normal LLM weights do not have) [2]:

| Precision | Dense peak |
|---|---|
| FP4 tensor, FP32 accumulate | 493.9 TFLOPS |
| FP8 tensor, FP16 accumulate | 246.9 TFLOPS |
| FP8 tensor, FP32 accumulate | 123.5 TFLOPS |
| FP16 / BF16 tensor, FP32 accumulate | 61.7 TFLOPS |
| FP16 tensor, FP16 accumulate | 123.5 TFLOPS |
| INT8 tensor | 246.9 TOPS |
| FP32 (non-tensor) | 30.9 TFLOPS |

NVIDIA's marketing figure of 988 AI TOPS [1] matches the sparse FP4 rate in the whitepaper (987.8) [2], so it is not a dense number. The tensor cores support FP16, BF16, TF32, INT8, FP8 and the new FP4 and FP6 formats [2].

## 2. Physical decode ceilings

### 2.1 Method

When one user generates one token at a time, the GPU has to read every weight the model uses once per token. Reading memory is the bottleneck, not math. So:

`ceiling tok/s = memory bandwidth / bytes read per token`

Bytes read per token were taken from the real GGUF files, not from rules of thumb. We read each file's tensor table over HTTP (the first 24 MB of the file) and added up the stored bytes of every tensor [17][18][19][20][21]. Two details matter:

- **Untied models** (Qwen3-8B, Qwen3-14B, Llama 3.1 8B) have a separate input embedding table. Decode reads only one row of it per token, so we subtract it.
- **Tied models** (Qwen3-4B, Gemma 3 12B) reuse the embedding table as the output layer, which is read in full every token, so nothing is subtracted.

This is the ceiling at zero context. Every token of context adds the KV cache to the bytes read. With an FP16 KV cache the extra bytes per context token are `2 (K and V) × layers × KV heads × head dim × 2 bytes`. Gemma 3 uses a 1,024-token sliding window on 5 of every 6 layers [40], so only its 8 global layers (48 / 6) read the full context.

Bandwidth used: 672 GB/s = 672 × 10⁹ bytes/s [2]. That is the spec peak. Real kernels never reach 100% of it, so we will measure achievable bandwidth on the actual card before claiming a percentage.

### 2.2 Results (4-bit = Q4_K_M, 8-bit = Q8_0)

| Model | Quant | File size (bytes) | Bytes read per token | Ceiling at 0 ctx | at 4k ctx | at 16k ctx | at 32k ctx | Fits in 12 GB? |
|---|---|---|---|---|---|---|---|---|
| Qwen3-4B | Q4_K_M | 2,497,280,256 | 2,491,323,904 | **269.7** | 217.1 | 136.9 | 91.8 | yes |
| Qwen3-4B | Q8_0 | 4,280,404,704 | 4,274,448,384 | **157.2** | 137.7 | 100.4 | 73.8 | yes |
| Qwen3-8B | Q4_K_M | 5,027,783,488 | 4,671,766,528 | **143.8** | 127.4 | 94.8 | 70.7 | yes |
| Qwen3-8B | Q8_0 | 8,709,518,112 | 8,042,336,256 | **83.6** | 77.7 | 64.3 | 52.2 | yes |
| Qwen3-14B | Q4_K_M | 9,001,752,960 | 8,558,218,240 | **78.5** | 72.8 | 59.8 | 48.3 | yes, up to about 16k context |
| Qwen3-14B | Q8_0 | 15,698,533,728 | 14,866,042,880 | 45.2 | 43.3 | 38.3 | 33.2 | **no** (file alone is 15.7 GB) |
| Llama 3.1 8B | Q4_K_M | 4,920,739,232 | 4,617,396,480 | **145.5** | 130.4 | 99.3 | 75.4 | yes |
| Llama 3.1 8B | Q8_0 | 8,540,775,840 | 7,974,764,800 | **84.3** | 79.0 | 66.4 | 54.8 | yes |
| Gemma 3 12B | Q4_K_M | 7,300,575,264 | 7,294,024,704 | **92.1** | 85.1 | 77.2 | 68.7 | yes |
| Gemma 3 12B | Q8_0 | 12,509,949,984 | 12,503,399,424 | 53.7 | 51.3 | 48.3 | 44.8 | **no** in practice (12.5 GB of weights, nothing left for KV cache or the Windows desktop) |

Worked example, Qwen3-8B Q4_K_M: tensors total 5,021,827,072 bytes, input embedding (Q4_K) is 350,060,544 bytes, so 5,021,827,072 − 350,060,544 = 4,671,766,528 bytes per token. 672,000,000,000 / 4,671,766,528 = 143.8 tok/s. At 4k context the KV cache adds 4,096 × (2 × 36 × 8 × 128 × 2) = 4,096 × 147,456 = 603,979,776 bytes, so 672 × 10⁹ / (4,671,766,528 + 603,979,776) = 127.4 tok/s.

File sizes are from the Hugging Face repos [17][18][19][20][21]; layer counts and head shapes are from each model's config. The Qwen3-14B "fits up to about 16k" note follows from 9.0 GB of weights plus 16,384 × 163,840 = 2.68 GB of FP16 KV cache; Hardware Corner also lists 16k as the largest context it ran for Qwen3-14B on this card [3].

### 2.3 Native Blackwell formats (what an engine could do that GGUF does not)

If every matmul weight were stored as NVFP4 (4-bit values plus one FP8 scale per 16 values = 4 + 8/16 = 4.5 bits per weight) or FP8 (8 bits per weight), the ceilings would be:

| Model | Weights read per token (params) | NVFP4 ceiling | FP8 ceiling |
|---|---|---|---|
| Qwen3-4B | 4,022,468,096 | 297.0 | 167.1 |
| Qwen3-8B | 7,568,405,504 | 157.8 | 88.8 |
| Qwen3-14B | 13,990,394,880 | 85.4 | 48.0 (does not fit: 14.8 GB) |
| Llama 3.1 8B | 7,504,924,736 | 159.2 | 89.5 |
| Gemma 3 12B | 11,765,788,416 | 101.5 | 57.1 (11.8 GB, too tight) |

Arithmetic: NVFP4 bytes = params × 4.5 / 8. For Qwen3-8B: 7,568,405,504 × 0.5625 = 4,257,228,096 bytes, and 672 × 10⁹ / 4,257,228,096 = 157.8 tok/s. These assume the output layer is also FP4, which costs quality; keeping it at 8 bits lowers the ceiling slightly.

### 2.4 Prefill ceilings (compute bound)

Reading a prompt is a batch of matrix multiplies, about 2 FLOPs per weight per token (attention adds more at long context, ignored here). Using the dense peaks from section 1 [2]:

| Model | FLOPs per token (2 × params) | FP16, FP32 acc (61.7 T) | FP8 or FP16-acc (123.5 T) | INT8 (246.9 T) | FP4 (493.9 T) | Best published |
|---|---|---|---|---|---|---|
| Qwen3-4B | 8.04 × 10⁹ | 7,669 | 15,351 | 30,690 | 61,393 | none |
| Qwen3-8B | 15.14 × 10⁹ | 4,076 | 8,159 | 16,311 | 32,629 | 3,487.7 at 4k [3] |
| Qwen3-14B | 27.98 × 10⁹ | 2,205 | 4,414 | 8,824 | 17,651 | 2,144.2 at 4k [3] |
| Llama 3.1 8B | 15.01 × 10⁹ | 4,111 | 8,228 | 16,449 | 32,905 | 2,927 [6] |
| Gemma 3 12B | 23.53 × 10⁹ | 2,622 | 5,248 | 10,492 | 20,989 | none |

Prefill has more headroom than decode: the best published Qwen3-8B prefill is 3,487.7 / 16,311 = 21% of the INT8 ceiling.

## 3. What others get on an RTX 5070 for local LLMs

### 3.1 Measured on an RTX 5070 (the five target models)

| Model | Quant | Runtime | Prompt / context | Decode tok/s | Prefill tok/s | Source | Date |
|---|---|---|---|---|---|---|---|
| Qwen3-4B | Q4_K_M | llama.cpp (GPU Battle harness) | not disclosed | **180.21** | n/a | [8] | 2026-07-11 |
| Qwen3-8B | Q4_K_M | llama.cpp (GPU Battle) | not disclosed | **109.63** | n/a | [8] | 2026-07-11 |
| Qwen3-8B | Q4_K_XL | llama.cpp llama-bench, Linux | 4k depth | 85.8 | **3,487.7** | [3][4] | Mar 2026 |
| Qwen3-8B | Q4_K_XL | llama.cpp llama-bench | 16k depth | **59.1** | 1,600.8 | [3] | Mar 2026 |
| Qwen3-8B | Q4_K_XL | llama.cpp llama-bench | 32k depth | **43.6** | 898.8 | [3] | Mar 2026 |
| Qwen3-14B | Q4_K_M | llama.cpp (GPU Battle) | not disclosed | **60.87** | n/a | [8] | 2026-07-11 |
| Qwen3-14B | Q4_K_XL | llama.cpp llama-bench | 4k depth | 54.2 | **2,144.2** | [3] | Mar 2026 |
| Qwen3-14B | Q4_K_XL | llama.cpp llama-bench | 16k depth | **40.6** | 1,315.2 | [3] | Mar 2026 |
| Llama 3.1 8B | Q4_K_M | llama.cpp (GPU Battle) | not disclosed | **119.87** | n/a | [8] | 2026-07-11 |
| Llama 3.1 8B | Q4_K_M | LocalScore | LocalScore suite, TTFT 477 ms | 55.9 | **2,927** | [6] | not shown |
| Gemma 3 12B | any | any | | **no measurement found** | | | |

Hardware Corner's method: llama.cpp `llama-bench` on Ubuntu 24.04, CUDA 12.8, Q4_K_XL for every run [4]. Its pages label the quant "Q4_K" [3]. GPU Battle runs llama.cpp with Q4_K_M on rented cloud GPUs and logs power per run; it measured Qwen3-4B at 132 W and Llama 3.1 8B at 210 W [8].

Warning on spread: for the same card, model and quant, GPU Battle reports 119.87 tok/s [8] and LocalScore reports 55.9 tok/s [6] for Llama 3.1 8B Q4_K_M. 119.87 / 55.9 = 2.14x. Published numbers are not controlled experiments. To claim a win we must run the competitor ourselves on the same PC (section 3.5).

### 3.2 Same card, other models (shows what each runtime does on this exact GPU)

| Model | Quant | Runtime | Shape | Decode tok/s | Prefill tok/s | Source | Date |
|---|---|---|---|---|---|---|---|
| Llama 2 7B (llama.cpp's standard test) | Q4_0 | llama.cpp CUDA, flash attn on, WSL | tg128 / pp512 | 128.21 | 5,783.44 | [10] | 2025-08-20 |
| Gemma 3 4B | Q4_K_M | llama.cpp CUDA (build 1a68ec9) | tg128 / pp512 | 177.1 | 9,445.5 | [11] | site run |
| Qwen2.5-Coder 7B | Q4_K_M | llama.cpp Vulkan (b9174) | 49-token chat | 120.7 | 1,531.0 | [12] | site run |
| Qwen3.5 9B | Q4_K_M | **Ollama 0.32.14, Windows 11** | 4k context, ~300-word answer | 104.58 | 297.44 (very short prompt) | [13] | 2026-08 |
| Gemma 4 12B (not Gemma 3) | Q4_K_M | llama.cpp (GPU Battle) | not disclosed | 69.06 | n/a | [8] | 2026-07-11 |
| Qwen2.5 14B | Q4_K_M | LocalScore | LocalScore suite | 20.8 | 1,264 | [6] | not shown |

Efficiency check on the reference model: llama-bench reports Llama 2 7B Q4_0 as 3.56 GiB, about 3.8225 × 10⁹ bytes [10]. Assuming its input embedding (32,000 × 4,096 values) is stored as Q4_0, that table is 73,728,000 bytes, leaving about 3.7488 × 10⁹ bytes read per token. 672 × 10⁹ / 3.7488 × 10⁹ = 179.3 tok/s ceiling, and 128.21 / 179.3 = 71.5% of ceiling.

### 3.3 Fallback cards (clearly not an RTX 5070)

The RTX 5070 Ti has 896 GB/s; 896 / 672 = 1.33. On llama.cpp's standard test the 5070 Ti decodes 176.85 tok/s [10] against 127.54 on the 5070 at the same flash-attention-off setting [10]; 176.85 / 127.54 = 1.39, close to the bandwidth ratio. Use these only to see what other runtimes do, not as 5070 targets.

| Card | Model | Quant | Runtime | Shape | Decode tok/s | Prefill tok/s | Source |
|---|---|---|---|---|---|---|---|
| RTX 5070 Ti | Qwen3-8B | AWQ (W4A16) | **vLLM 0.13.0** | concurrency 1 | 97.8 (mean); 122.0 in the same post's sweep table | n/a | [14] |
| RTX 5070 Ti | Qwen3-8B | NVFP4 | **vLLM 0.13.0** | concurrency 1 | 98.4 | n/a | [14] |
| RTX 5070 Ti | Qwen3-8B | Q4_K_XL | llama.cpp | 4k depth | 120.5 | 5,557.3 | [5] |
| RTX 5070 Ti | Qwen3-14B | Q4_K_XL | llama.cpp | 4k depth | 74.3 | 3,264.7 | [5] |
| RTX 5070 Ti | Qwen3-4B / 8B / 14B / Llama 3.1 8B | Q4_K_M | llama.cpp (GPU Battle) | not disclosed | 231.69 / 139.27 / 79.02 / 153.23 | n/a | [9] |
| RTX 5070 Ti | Llama 3.1 8B | Q4_K_M | LocalScore | suite | 65.5 | 3,657 | [7] |

### 3.4 Not found (searched 2026-10-19)

No published RTX 5070 number was found for any of the five target models on **LM Studio, ExLlamaV2, ExLlamaV3, vLLM, TensorRT-LLM or MLC-LLM**, nor for **Ollama** on the target models. No published RTX 5070 number exists for **Gemma 3 12B** in any runtime, and none for any **8-bit** run of the target models. For those cells the target is: run the best available runtime ourselves on the same PC and beat it.

### 3.5 What counts as "beating" a target

1. Same PC, same driver, same model file family, same context depth, back to back.
2. Competitors: latest llama.cpp CUDA build (`llama-bench -p 512 -n 128 -d 0,4096,16384 -fa 1`), Ollama and LM Studio on Windows.
3. Report decode and prefill separately, plus power draw, and keep the raw logs as receipts.
4. Our number must beat both the published target in section 0 and the same-machine competitor run.

## 4. Speech to text on RTX 5070-class cards

| Card | Engine | Model | Mode | Audio | Wall time | Speed | Source |
|---|---|---|---|---|---|---|---|
| **RTX 5070** | faster-whisper 1.2.1 (CTranslate2 4.8.1), Windows | large-v3, fp16 | BatchedInferencePipeline, batch 16, VAD, word timestamps, Chinese | 141.4 min | 196.7 s | **43.1x real time (THE TARGET)** | [15] |
| RTX 5070 | same | large-v3, fp16 | default sequential `transcribe()` | 141.4 min | 1,589.5 s | 5.3x real time | [15] |
| RTX 3070 Ti 8 GB (older card, reference) | faster-whisper | large-v2, int8 | batch 8, beam 5 | 13 min | 16 s | 48.75x (780 s / 16 s) | [16] |
| RTX 3070 Ti 8 GB (reference) | faster-whisper | large-v2, fp16 | batch 8, beam 5 | 13 min | 17 s | 45.9x (780 / 17) | [16] |
| RTX 3070 Ti 8 GB (reference) | whisper.cpp, flash attention | large-v2, fp16 | beam 5 | 13 min | 65 s | 12.0x (780 / 65) | [16] |
| RTX 3070 Ti 8 GB (reference) | openai/whisper | large-v2, fp16 | beam 5 | 13 min | 143 s | 5.5x (780 / 143) | [16] |

The RTX 5070 run used real 2-hour livestream audio with word timestamps; the 3070 Ti figures use a 13-minute clip, so the two are not directly comparable. The batched pipeline was about 8x faster than sequential on the same 5070 (1,589.5 / 196.7 = 8.1) [15]. Not found: any RTX 5070 measurement for large-v3-turbo, distil-whisper, whisper.cpp, or NVIDIA Parakeet. Our speech target: beat 43.1x on large-v3 at equal quality, and publish a large-v3-turbo number, since none was found.

## 5. Gaming: what actually raises FPS on a 5070

Baseline: across 16 games at 1440p the RTX 5070 is 1% faster than an RTX 4070 Super [27], and TechPowerUp measured it 22% faster than an RTX 4070 in 1440p raster [24]. Its 12 GB of VRAM runs out in some 4K and ray-traced games, often when frame generation is on [25][26][27].

### 5.1 Lever by lever

| Lever | Measured effect on a 5070 (or nearest) | Source | Safe for an automated tool? |
|---|---|---|---|
| **DLSS Super Resolution mode** | Cyberpunk 2077 RT: native 43 fps / 47 ms; Quality 71 fps (+65%); Balanced 84 (+95%); Performance 100 (+133%); Ultra Performance 147 (+242%); latency falls to 33 / 29 / 26 / 22 ms | [23] | **Recommend only.** Per-game in-game setting; image quality is a taste call. NVIDIA's own app offers per-game DLSS overrides [38]. |
| DLSS 4 Transformer vs older CNN model | Same test: Quality 71 vs 74 fps (−4.1%), Performance 100 vs 105 (−4.8%) [23]; ~7% average cost at the same mode on an RTX 5080 [28]; 3 to 5% on an RTX 4080 [29]; TechPowerUp calls it "roughly 10%" [24]. DLSS 4 Performance ran 14% faster than DLSS 3 Quality on average [28]. | [23][28][29] | Recommend only. |
| **DLSS 4 Multi Frame Generation** | Cyberpunk RT, no upscaling: 43 → 84 (2x) → 121 (3x) → 154 fps (4x), latency 47 → 50 → 57 → 54 ms [23]. Cyberpunk path tracing 4K with DLSS Performance: 40 → 122 fps at 4x, latency 51 → 72 ms [22]. Alan Wake 2 4K: 114 → 146 ms at 4x, "unreasonably high" [22]. Hogwarts Legacy 4K: 100 → 234 fps, 25.6 → 38.9 ms [25]. With VRAM full, Gamers Nexus saw 500 to 720 ms latency after 5 minutes [26]. Works well from a base of about 40 to 50 fps [24]. Frame generation does not lower latency [27]. | [22][23][24][25][26][27] | **Recommend only**, and only when base fps is high enough and VRAM has headroom. Never force on. Requires HAGS [32]. |
| NVIDIA Smooth Motion | Driver-level frame generation for RTX 50 games without DLSS Frame Generation [38]. No reviewer measurement collected. | [38] | Recommend only, per game. |
| **NVIDIA Reflex** | Latency, not FPS. NVIDIA's own 5070 example (THE FINALS, 4K): 56 ms → 27 ms with Reflex (−52%), 14 ms with Reflex 2 Frame Warp (−75%) [31]. Vendor figure, not independent. | [31] | Recommend "On" in each game; no API to set it. |
| **Resizable BAR** | No RTX 50 test found. On an RTX 3080 across 20 games: 0% (1080p), 1% (1440p), 2% (4K) average; best +9%, worst −10% (Watch Dogs Legion, 1080p). NVIDIA enables it per game by whitelist. | [30] | **Detect and report only.** Turning it on is a BIOS change. Do not force it on for non-whitelisted games. |
| **Manual overclock** | +13% real-life performance on the 5070 Founders Edition (about 2,800 → 3,150 MHz); memory OC hits a driver cap of +375 MHz | [24] | **No.** Stability differs chip to chip; crashes and data loss risk. |
| **Undervolt / power limit** | One user's 5070, Heaven benchmark: 900 mV undervolt cut FPS 255.5 → 242.4 (−5.1%) while max board power fell 253.1 → 161.0 W (−92 W) and GPU temperature fell 10.1 °C. Community report, single card. | [39] | **No for undervolt.** It saves power and noise, it does not raise FPS. A power-limit cap could be an opt-in "quiet mode". |
| **Hardware-accelerated GPU scheduling (HAGS)** | Required for DLSS Frame Generation to work [32]. No reputable measured FPS gain found. | [32] | **Safe to check**; enabling needs a reboot, so ask first. |
| **Windows Game Mode** | RTX 4060 Laptop, MakeUseOf testing (reported via WindowsForum): GPU-bound Cyberpunk avg 82.4 → 82.9 fps (no change); CS2 1% low 142.3 → 154.6 fps (+8.6%); Forza Horizon 5 with Edge and Discord open, 1% low 48.2 → 71.5 fps (+48%). | [33] | **Safe.** Make sure it is on. |
| **Background apps** | The only measured data found is the Forza Horizon 5 run above, with Edge and Discord open [33]. No reputable reviewer isolated background apps on a 5070. | [33] | Report heavy CPU, GPU and VRAM users; **ask before closing anything; never kill processes silently.** |
| **Monitor refresh mismatch** | Not an FPS gain but a visible one: Windows does not always switch a new monitor to its top refresh rate [35]. A 60 Hz setting shows at most 60 updates per second [34], so a 144 Hz panel left at 60 Hz throws away 84 of 144 possible refreshes each second (144 − 60). | [34][35] | **Safe to detect**; change only with the user's OK. |
| **Driver: Power management mode** | "Prefer maximum performance" only helps when the GPU is wrongly lowering clocks [36]. | [36] | Safe if the tool saves and can restore the old driver profile. Not a default. |
| **Driver: G-SYNC + V-SYNC + frame cap** | Blur Busters: cap at least 3 fps below max refresh; with Reflex or Low Latency Mode "Ultra" the cap is applied automatically [37]. Removes tearing and latency spikes, not an FPS gain. | [37] | Safe if reversible and backed up. |
| **VRAM pressure** | The 12 GB buffer is exceeded in some games with RT and frame generation at 4K (Indiana Jones, Spider-Man 2, Ratchet and Clank) [25]; performance then collapses [26][27]. | [25][26][27] | **Safe to watch**; warn and suggest lower textures or no frame generation. |

### 5.2 What the tool may set by itself

- **Yes, automatically:** nothing that changes BIOS, clocks, voltages or game files. Reading state is always fine: refresh rate, HAGS, Game Mode, ReBAR status, VRAM use, background load.
- **Yes, with one click from the user and a saved undo:** Windows Game Mode on, refresh rate to the monitor's maximum, HAGS on (reboot), driver frame cap and G-SYNC profile.
- **Recommend only:** DLSS mode, frame generation, Smooth Motion, Reflex, closing apps.
- **Never:** overclocking, undervolting, forcing Resizable BAR, killing processes.

## 6. Gaps and caveats

- GPU Battle does not publish prompt length or llama.cpp version per row [8], and its targets are the highest numbers found, not averages. A different published run for Llama 3.1 8B is 2.14x lower [6].
- Hardware Corner uses Unsloth Q4_K_XL files [4], which differ in size from the Q4_K_M files used for the ceiling math, so its percent-of-ceiling is approximate.
- All ceilings assume the 672 GB/s spec peak. We have not yet measured achievable bandwidth on the brother's card.
- No independent (non-NVIDIA) Reflex measurement on a 5070 was collected. No Resizable BAR test on any RTX 50 card was found.
- The Game Mode numbers come from a secondary summary of MakeUseOf's testing [33]; the original article URL was not retrieved.
- The undervolt data is one user's card [39].

## Sources

[1] https://www.nvidia.com/en-us/geforce/graphics-cards/50-series/rtx-5070-family : NVIDIA: GeForce RTX 5070 Family specs
[2] https://images.nvidia.com/aem-dam/Solutions/geforce/blackwell/nvidia-rtx-blackwell-gpu-architecture.pdf : NVIDIA RTX Blackwell GPU Architecture whitepaper (Table 6, GB205 / RTX 5070)
[3] https://www.hardware-corner.net/gpu-llm-benchmarks/rtx-5070 : Hardware Corner: RTX 5070 local LLM benchmarks
[4] https://www.hardware-corner.net/gpu-ranking-local-llm : Hardware Corner: GPU ranking for local LLMs (method: llama-bench, CUDA 12.8, Q4_K_XL)
[5] https://www.hardware-corner.net/gpu-llm-benchmarks/rtx-5070-ti : Hardware Corner: RTX 5070 Ti local LLM benchmarks
[6] https://www.localscore.ai/accelerator/168 : LocalScore: NVIDIA GeForce RTX 5070 results
[7] https://www.localscore.ai/accelerator/160 : LocalScore: NVIDIA GeForce RTX 5070 Ti results
[8] https://gpubattle.com/ai/geforce-rtx-5070 : GPU Battle: GeForce RTX 5070 AI benchmarks (llama.cpp Q4_K_M, measured 2026-07-11)
[9] https://gpubattle.com/ai/geforce-rtx-5070-ti : GPU Battle: GeForce RTX 5070 Ti AI benchmarks
[10] https://github.com/ggml-org/llama.cpp/discussions/15013 : llama.cpp discussion 15013: RTX 5070 llama-bench (Llama 2 7B Q4_0)
[11] https://calebcoffie.com/benchmarks/gemma-3-4b-it-gguf : Caleb Coffie: Gemma 3 4B runs incl. RTX 5070 llama.cpp CUDA
[12] https://calebcoffie.com/benchmarks/qwen2-5-coder-7b-instruct-gguf : Caleb Coffie: Qwen2.5-Coder 7B runs incl. RTX 5070 llama.cpp Vulkan
[13] https://github.com/Pyghast/local-llm-benchmarks : Pyghast: Ollama benchmarks on RTX 5070 (Windows 11)
[14] https://kr-lsb.github.io/posts/inferbench-awq-vs-nvfp4 : kr-lsb: vLLM AWQ vs NVFP4 on RTX 5070 Ti
[15] https://github.com/kildren-coder/story-machine/issues/9 : kildren-coder: faster-whisper on RTX 5070, measured speed (issue #9 comments)
[16] https://github.com/SYSTRAN/faster-whisper : SYSTRAN faster-whisper README benchmark (RTX 3070 Ti)
[17] https://huggingface.co/Qwen/Qwen3-4B-GGUF : Qwen3-4B-GGUF (file sizes, tensor table)
[18] https://huggingface.co/Qwen/Qwen3-8B-GGUF : Qwen3-8B-GGUF
[19] https://huggingface.co/Qwen/Qwen3-14B-GGUF : Qwen3-14B-GGUF
[20] https://huggingface.co/bartowski/Meta-Llama-3.1-8B-Instruct-GGUF : bartowski Meta-Llama-3.1-8B-Instruct-GGUF
[21] https://huggingface.co/bartowski/google_gemma-3-12b-it-GGUF : bartowski google_gemma-3-12b-it-GGUF
[22] https://www.digitalfoundry.net/articles/digitalfoundry-2025-nvidia-geforce-rtx-5070-review?page=8 : Digital Foundry: RTX 5070 review, DLSS 4 and path tracing page
[23] https://www.techpowerup.com/review/nvidia-geforce-rtx-5070-founders-edition/39.html : TechPowerUp: RTX 5070 FE review, DLSS 4 page
[24] https://www.techpowerup.com/review/nvidia-geforce-rtx-5070-founders-edition/46.html : TechPowerUp: RTX 5070 FE review, conclusion (OC, transformer cost)
[25] https://www.kitguru.net/components/graphic-cards/dominic-moass/nvidia-rtx-5070-review/all/1 : KitGuru: RTX 5070 review (MFG latency)
[26] https://gamersnexus.net/gpus/nvidia-selling-lies-rtx-5070-founders-edition-review-benchmarks : Gamers Nexus: RTX 5070 FE review
[27] https://www.techspot.com/review/2960-nvidia-geforce-rtx-5070 : TechSpot (Hardware Unboxed): RTX 5070 review
[28] https://www.techspot.com/article/2957-nvidia-dlss-4-vs-dlss-3 : TechSpot (Hardware Unboxed): DLSS 4 vs DLSS 3
[29] https://www.techpowerup.com/review/nvidia-dlss-4-transformers-image-quality : TechPowerUp: DLSS 4 Transformer review
[30] https://www.techspot.com/review/2234-nvidia-resizable-bar : TechSpot (Hardware Unboxed): Nvidia Resizable BAR tested (RTX 3080, 20 games)
[31] https://www.nvidia.com/en-us/geforce/news/reflex-2-even-lower-latency-gameplay-with-frame-warp : NVIDIA: Reflex 2 with Frame Warp (RTX 5070 example)
[32] https://www.tomshardware.com/pc-components/gpu-drivers/amd-finally-adds-hardware-accelerated-gpu-scheduling-support-to-some-of-its-gpus-three-years-after-the-feature-first-debuted-in-windows : Tom's Hardware: DLSS frame generation requires HAGS
[33] https://windowsforum.com/news/windows-11-game-mode-raises-1-lows-not-average-fps.437707 : WindowsForum summary of MakeUseOf Game Mode testing (secondary)
[34] https://support.microsoft.com/en-us/windows/hardware/display-graphics/change-the-refresh-rate-on-your-monitor-in-windows : Microsoft: change the refresh rate on your monitor
[35] https://www.newegg.com/insider/how-to-adjust-refresh-rate-settings-in-windows-and-gpu-control-panels : Newegg Insider: refresh rate stuck at 60Hz
[36] https://nvidia.custhelp.com/app/answers/detail/a_id/3130/~/setting-power-management-mode-from-normal-to-maximum-performance : NVIDIA support: Power management mode
[37] https://blurbusters.com/gsync/gsync101-input-lag-tests-and-settings/14 : Blur Busters: G-SYNC 101 optimal settings
[38] https://www.nvidia.com/en-us/geforce/news/nvidia-app-update-dlss-overrides-and-more : NVIDIA: App update, DLSS overrides and Smooth Motion
[39] https://www.reddit.com/r/nvidia/comments/1ui60cu/undervolt_on_rtx_5070_is_amazing : Reddit r/nvidia: RTX 5070 undervolt (single user, community)
[40] https://huggingface.co/unsloth/gemma-3-12b-it : Gemma 3 12B config (sliding_window 1024, pattern 6)

Source [10] covers two comments in the same discussion: RTX 5070 at https://github.com/ggml-org/llama.cpp/discussions/15013#discussioncomment-14161275 and RTX 5070 Ti at https://github.com/ggml-org/llama.cpp/discussions/15013#discussioncomment-15183261. Verbatim supporting quotes for every source are in `evidence/targets/sources_ledger.json`.
