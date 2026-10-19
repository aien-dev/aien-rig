#!/usr/bin/env python3
"""Put the Windows release zip together.

    python tools/package.py --bin-dir target/x86_64-pc-windows-msvc/release \
        --out dist --version v0.1.0 [--require-all]

The zip is always named aien-rig-windows-x64.zip so that
https://github.com/aien-dev/aien-rig/releases/latest/download/aien-rig-windows-x64.zip
always points at the newest release.
"""
import argparse
import hashlib
import pathlib
import sys
import zipfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
ZIP_NAME = "aien-rig-windows-x64.zip"
PROGRAMS = ["aien-rig.exe", "rig-tune.exe", "rig-create.exe"]
OPTIONAL = ["rig-listen.exe"]  # speech to text; included when it built
DOCS = [("README-FIRST.txt", "README-FIRST.txt"), ("LICENSE", "LICENSE.txt")]


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin-dir", action="append", required=True, help="folder(s) holding the built .exe files")
    ap.add_argument("--out", default="dist")
    ap.add_argument("--version", default="dev")
    ap.add_argument("--commit", default="")
    ap.add_argument("--require-all", action="store_true", help="fail when a program is missing (release tags)")
    a = ap.parse_args(argv)

    found, missing = {}, []
    for exe in PROGRAMS:
        for d in a.bin_dir:
            p = pathlib.Path(d) / exe
            if p.is_file():
                found[exe] = p
                break
        else:
            missing.append(exe)
    for exe in OPTIONAL:
        for d in a.bin_dir:
            p = pathlib.Path(d) / exe
            if p.is_file():
                found[exe] = p
                break
    for m in missing:
        print(f"{'error' if a.require_all else 'warning'}: {m} was not built", file=sys.stderr)
    if "aien-rig.exe" not in found or (a.require_all and missing):
        return 1

    out = pathlib.Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    zpath = out / ZIP_NAME
    version_txt = (
        f"AIEN Rig {a.version}\ncommit {a.commit}\n"
        f"programs: {', '.join(found)}\n"
        + (f"not in this build: {', '.join(missing)}\n" if missing else "")
    )
    with zipfile.ZipFile(zpath, "w", zipfile.ZIP_DEFLATED) as z:
        for exe, p in found.items():
            z.write(p, f"AIEN Rig/{exe}")
        for src, dst in DOCS:
            z.write(ROOT / src, f"AIEN Rig/{dst}")
        z.writestr("AIEN Rig/VERSION.txt", version_txt)
    digest = hashlib.sha256(zpath.read_bytes()).hexdigest()
    (out / (ZIP_NAME + ".sha256")).write_text(f"{digest}  {ZIP_NAME}\n", encoding="utf-8")
    print(f"{zpath} sha256 {digest}")
    print(version_txt)
    return 0


if __name__ == "__main__":
    sys.exit(main())
