# Transcript JSON (aien-rig-transcript/1)

This is the file the Rig speech engine writes and `rig-create` reads. Any tool
can produce it. Times are seconds from the start of the video, as numbers.

```json
{
  "schema": "aien-rig-transcript/1",
  "language": "en",
  "duration": 478.0,
  "source": "castle-build.mp4",
  "segments": [
    {
      "start": 0.0,
      "end": 5.8,
      "text": "What's up everybody, welcome back to the channel.",
      "words": [
        { "start": 0.0, "end": 0.32, "word": "What's", "prob": 0.98 },
        { "start": 0.32, "end": 0.41, "word": "up", "prob": 0.99 }
      ]
    }
  ]
}
```

## Fields

| Field | Required | Meaning |
|---|---|---|
| `schema` | yes for writers | Always `"aien-rig-transcript/1"`. Readers do not reject files without it. |
| `language` | no | Spoken language, ISO 639-1 code (`"en"`). |
| `duration` | no | Length of the whole video in seconds. If missing, the end of the last segment is used. Write it when you know it: the last chapter needs 10 seconds before the video ends. |
| `source` | no | Name of the media file the transcript came from. |
| `segments` | yes | Speech in time order. |
| `segments[].start` | yes | Start in seconds (number). A timestamp string such as `"01:02.5"` is also accepted. |
| `segments[].end` | yes for writers | End in seconds. If missing, the next segment's start is used. |
| `segments[].text` | yes | What was said. Leading and trailing spaces are fine (Whisper adds a leading space). |
| `segments[].words` | no | Word times. Each word has `start`, `end`, `word`, and optionally `prob` (0 to 1). If a segment has no `text`, the words are joined to make it. |

Unknown fields are ignored, so writers may add their own (for example `id`,
`avg_logprob`, `no_speech_prob`).

## Guidance for the speech engine

- One segment per sentence or phrase, ideally 1 to 10 seconds. Very long
  segments make chapter and Shorts times less exact; very short ones are fine.
- Keep filler words ("um", "uh") in `text`. `rig-create` marks and removes them
  itself, and the counts are useful to the creator.
- Keep punctuation if the model gives it. Commas help find "you know," and
  ", like," fillers.
- Segments must not go backwards in time. Overlaps are tolerated.

## Also accepted

`rig-create` also reads, without conversion:

- Whisper / faster-whisper JSON (`{"text": ..., "segments": [{"start", "end", "text"}]}`).
- A bare JSON list of `{"start", "end", "text"}` objects.
- SubRip `.srt` and WebVTT `.vtt`, including YouTube auto-captions.
- YouTube `.json3` caption files (`yt-dlp --write-auto-subs --sub-format json3`).
- Text copied from YouTube's "Show transcript" panel (`0:00` lines), or any
  text whose lines start with a timestamp.
- Plain text with no times at all. Times are then estimated at 2.6 words per
  second and every result says so.

A short real example is in `samples/speech-example.json`.
