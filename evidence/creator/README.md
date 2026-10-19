# rig-create evidence

Real runs of `rig-create` against our own model server (qwen3.8-27b-fp8 on the Spark, port 18091). Nothing here is mocked.

## 2026-10-19-e2e-18091 (passed)
- Input: `creator/samples/castle-build.srt` (7:58 captions, hand-written sample).
- Binary: source sha 7aef4de (`run-info.txt`), release build on the Spark.
- Result: exit 0, 4 parts read + 1 final request, 49 min 43 s wall clock, almost all of it waiting: the server answered "GPU busy" (503) while other jobs held the GPU lock, and `--busy-wait 14400` retried until the lock cleared. The tool itself used 0.4 s CPU and 2.6 MB RAM.
- Output (`output/`): 5 titles, description with chapters, tags, 4 Shorts picks, filler captions (12 fillers found), `result.json`. `warnings` is empty and the chapter check passes YouTube's rules.
- Spot check: all 4 Shorts hook quotes appear verbatim in the source captions; "41x41" and "3 days" in the titles are said in the source.
- `run-info.txt` and `stderr.txt` show the `server_process` line of whatever else was running on the box at that moment (an unrelated agent job); ignore it.

## 2026-10-19-e2e-attempt1-all-503 (failed, kept on purpose)
- Same command at sha 8c2c463 without a long busy wait. The server answered 503 "GPU lock held by other jobs for 300 s" on every try for 899 s and the tool gave up with exit 1. This is why `--busy-wait` exists (commit 7aef4de).
