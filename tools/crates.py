#!/usr/bin/env python3
"""List the Rust crates present in this checkout and the programs they make.

    python tools/crates.py              # human-readable
    python tools/crates.py --json plan.json

The app is in the root workspace. tuner/, creator/ and speech/ are standalone
crates (excluded from the root workspace) built with --manifest-path.
"required" crates must build for a release tag; the others are best effort.
"""
import argparse
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
STANDALONE = [
    # (folder, program, required for a release)
    ("tuner", "rig-tune", True),
    ("creator", "rig-create", True),
    ("speech", "rig-listen", False),
]


def plan(root):
    out = {"workspace": ["app"], "standalone": [], "missing": []}
    for folder, program, required in STANDALONE:
        entry = {"folder": folder, "program": program, "required": required}
        if (root / folder / "Cargo.toml").is_file():
            out["standalone"].append(entry)
        else:
            out["missing"].append(entry)
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--json", help="write the plan as JSON to this file")
    ap.add_argument("--root", default=str(ROOT))
    a = ap.parse_args(argv)
    p = plan(pathlib.Path(a.root))
    print("workspace: ", ", ".join(p["workspace"]))
    print("standalone:", ", ".join(e["folder"] for e in p["standalone"]) or "(none)")
    print("missing:   ", ", ".join(e["folder"] for e in p["missing"]) or "(none)")
    if a.json:
        pathlib.Path(a.json).write_text(json.dumps(p, indent=2), encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
