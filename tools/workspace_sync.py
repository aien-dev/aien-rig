#!/usr/bin/env python3
"""Keep the root Cargo.toml workspace members in line with the crates present.

The planned members are app, tuner, creator and speech. Each stream builds its
crate on its own branch, so a checkout may hold only some of them, and Cargo
refuses a members list that names a folder without a Cargo.toml.

    python tools/workspace_sync.py            # show what would change
    python tools/workspace_sync.py --write    # rewrite the members line
    python tools/workspace_sync.py --write --report plan.json

A crate whose own Cargo.toml declares [workspace] is left out of the root
workspace (added to `exclude`) and listed under "standalone" in the report so
CI builds it on its own with --manifest-path.
"""
import argparse
import json
import pathlib
import re
import sys

PLANNED = ["app", "tuner", "creator", "speech"]
ROOT = pathlib.Path(__file__).resolve().parent.parent


def find_crates(root):
    """Planned folders with a Cargo.toml, split into members and standalone."""
    members, standalone = [], []
    for name in PLANNED:
        manifest = root / name / "Cargo.toml"
        if not manifest.is_file():
            continue
        text = manifest.read_text(encoding="utf-8")
        if re.search(r"^\s*\[workspace\]\s*$", text, re.M):
            standalone.append(name)
        else:
            members.append(name)
    return members, standalone


def toml_list(items):
    return "[" + ", ".join(json.dumps(i) for i in items) + "]"


def rewrite(text, members, standalone):
    new = re.sub(r"(?m)^members\s*=\s*\[[^\]]*\]", "members = " + toml_list(members), text, count=1)
    if re.search(r"(?m)^exclude\s*=", new):
        new = re.sub(r"(?m)^exclude\s*=\s*\[[^\]]*\]", "exclude = " + toml_list(standalone), new, count=1)
    elif standalone:
        new = re.sub(r"(?m)^(members\s*=.*)$", r"\1\nexclude = " + toml_list(standalone).replace("\\", "\\\\"), new, count=1)
    return new


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--write", action="store_true", help="rewrite Cargo.toml")
    ap.add_argument("--report", help="write a JSON build plan here")
    ap.add_argument("--root", default=str(ROOT))
    a = ap.parse_args(argv)
    root = pathlib.Path(a.root)
    cargo = root / "Cargo.toml"
    members, standalone = find_crates(root)
    if "app" not in members:
        print("warning: app/Cargo.toml is missing or standalone", file=sys.stderr)
    old = cargo.read_text(encoding="utf-8")
    new = rewrite(old, members, standalone)
    print("members:   ", ", ".join(members) or "(none)")
    print("standalone:", ", ".join(standalone) or "(none)")
    print("missing:   ", ", ".join(n for n in PLANNED if n not in members + standalone) or "(none)")
    if new != old:
        if a.write:
            cargo.write_text(new, encoding="utf-8")
            print("Cargo.toml updated")
        else:
            print("Cargo.toml would change (run with --write)")
    if a.report:
        pathlib.Path(a.report).write_text(
            json.dumps({"members": members, "standalone": standalone}, indent=2), encoding="utf-8"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
