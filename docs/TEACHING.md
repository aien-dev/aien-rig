# Learn the Rig status boundary

At `c1f49889e9b53c5507fe22fe6c19ede2ffb51660`, I described a proposed RTX 5070 local assistant and FPS tuner. [The stream plan](PLAN.md) assigns engine, speech, creator, tuner and app work to separate branches. On this default branch, each directory contains only a README saying "in progress". There is no inference or tuner source, executable, fixture, or `evidence/` here. The five branch names in the plan are work ownership, not a shipped flow.

The design separates model execution from speech, content assistance and reversible game tuning so each outcome can be measured on its own. That rationale is visible in [README](../README.md) and [plan](PLAN.md); it is not an implemented architecture here. Comparing throughput with llama.cpp and safe reversible FPS changes remain goals. A Spark result with a 12 GiB limit would not establish RTX 5070 behavior; the target needs a 5070 run with model, driver, game, settings, baseline, reversal and retained receipt.

## Fresh checkout exercise (offline, no GPU)

Prerequisites: Git and Python 3. Checkout `c1f49889e9b53c5507fe22fe6c19ede2ffb51660`, then run from repository root:

```sh
python3 - <<'PYEX'
from pathlib import Path
for part in ('engine','speech','creator','tuner','app'):
    files=sorted(p.name for p in Path(part).iterdir() if p.is_file())
    assert files == ['README.md'], (part,files)
    print(part+': plan only')
PYEX
```

Expected: five `plan only` lines, exit 0. This is a reproducible absence check, not a GPU benchmark. The negative case is any attempted build from `main`: there is no build manifest. A runnable engine route is blocked until source and fixtures merge. [Replay receipt](teaching-replay-2026-10-22.txt).


### Parent fresh-checkout replay

A separate coordination replay is retained in [the parent receipt](parent-replay-2026-10-22.txt). Its scope and observed exit are explicit; source review and final desk qualification remain separate. A recorded absence or expected failing baseline is not a passing implementation.
