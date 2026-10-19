# creator: the Rig YouTube helper

Give it your video's captions or transcript. In a minute or two it writes:

- **5 title ideas**
- **a description with chapters** (`00:00 Intro`, ...) that follows YouTube's rules, so chapters show up on your video
- **tags**, ready to paste
- **3 to 5 Shorts picks**: where to cut, the opening line, and why it works
- **clean captions**: "um", "uh" and stutters taken out, plus a copy with them marked and a list of where they are (handy for cutting)

Everything runs on your own PC through the Rig's local model. Nothing is uploaded.

## Use it

1. Get your video's captions. Any of these work:
   - the Rig speech engine's transcript (`.json`)
   - captions from YouTube Studio (Subtitles > your language > Download, `.srt` or `.vtt`)
   - YouTube auto-captions saved with yt-dlp (`.vtt` or `.json3`)
   - text copied from YouTube's "Show transcript" panel, saved as `.txt`
2. Run:

   ```
   rig-create my-video.srt
   ```

   To tell it about your channel (gives better titles and tags):

   ```
   rig-create my-video.srt --about "Minecraft survival builds with a time limit"
   ```

3. Open the new `my-video-rig` folder and open `youtube.txt`. Copy each part into YouTube.

Files in the results folder:

| File | What it is |
|---|---|
| `youtube.txt` | Everything, in the order YouTube asks for it. Start here. |
| `description.txt` | The description with chapters, ready to paste. |
| `tags.txt` | Tags, comma separated, ready to paste. |
| `captions.clean.srt` | Captions with filler words removed. Upload this to YouTube. |
| `captions.marked.srt` | Captions with filler words shown in `{curly brackets}`. |
| `fillers.txt` | Every filler word with its time, for cutting them in your editor. |
| `result.json` | Everything above in one file, for the Rig app. |

Only want clean captions? `rig-create my-video.srt --captions-only` needs no model at all.

`rig-create --help` lists every option.

## What it checks for you

The model makes suggestions; the helper checks them in code before you see them:

- Chapters start at `00:00`, there are at least 3, they go in order, and each is at least 10 seconds long. If the model gets this wrong, the helper fixes it.
- Shorts picks start and end on whole sentences, last 15 to 60 seconds, and do not overlap.
- Titles stay under YouTube's 100 characters; tags stay under YouTube's 500 character total; the description stays under 5000 characters; `<` and `>` (which YouTube rejects) are removed.
- Anything it could not do well is listed under PLEASE CHECK in `youtube.txt`.

Filler words: it only removes words that are almost never meaningful ("um", "uh", "erm",
"hmm", a short word said twice like "the the"). "you know", "I mean" and "like" are only
treated as fillers when commas set them apart, so auto-captions (no commas) keep them.

## Model server

By default it talks to the Rig engine at `http://127.0.0.1:18120/v1`. It works with any
server that speaks the OpenAI chat API (llama.cpp server, Ollama, LM Studio, vLLM):

```
rig-create my-video.srt --server http://127.0.0.1:8080/v1
```

You can also set `RIG_SERVER`, `RIG_MODEL` and `RIG_API_KEY`. Plain `http` only by default;
build with `--features tls` for an `https` server. A busy server (HTTP 503 or 429) is retried,
honoring `Retry-After`, for up to 15 minutes.

Long videos are split into parts that fit the model's context (`--context`, default 8192 tokens).
Each part is summarized, then the summaries are combined into one set of results (merged in
rounds first if they are still too long).

## For developers

- Library crate `aien_rig_creator`; the app can call `aien_rig_creator::run(&Job, progress)` or use
  the pieces (`transcript`, `filler`, `chapters`, `shorts`, `pipeline::create` with any `ChatModel`).
- Transcript JSON schema for the speech engine: [TRANSCRIPT.md](TRANSCRIPT.md).
- Tests: `cargo test` (no model or GPU needed; a fake model and a fake HTTP server stand in).
  CI runs them on Windows and Linux: `.github/workflows/creator.yml`.
- The crate has no `[workspace]` table on purpose. If a top-level workspace is added, either list
  `creator` as a member or add it to `exclude`.
- `samples/castle-build.srt` is a self-made transcript (written for this project, free to reuse)
  of an 8 minute Minecraft build video, with filler words left in.
- End-to-end run against a real model: `evidence/creator/`.

License: AGPL-3.0.
