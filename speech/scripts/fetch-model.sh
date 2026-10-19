#!/usr/bin/env bash
# Download a Hugging Face model at a pinned revision into ~/models/<name>,
# check every file's size and every LFS file's sha256 against the Hugging Face
# API listing, and write <name>.SOURCE and <name>.sha256 next to it.
#
# Usage: fetch-model.sh <repo> <revision> <name> <license-note>
# Example:
#   fetch-model.sh openai/whisper-large-v3-turbo 41f01f3fe87f28c78e2fbf8b568835947dd65ed9 \
#       whisper-large-v3-turbo-41f01f3f "MIT (model card front matter: license: mit)"
set -euo pipefail
repo="$1"; rev="$2"; name="$3"; license="${4:-see model card}"
models="${MODELS_DIR:-$HOME/models}"
dst="$models/$name"
mkdir -p "$dst"
listing="$(mktemp)"
curl -sf "https://huggingface.co/api/models/$repo/revision/$rev?blobs=true" > "$listing"
start=$(date -u +%Y-%m-%dT%H:%M:%SZ)
t0=$(date +%s)
report="$(mktemp)"
total=0; nfiles=0; ok=1
while IFS=$'\t' read -r f size sha; do
  case "$f" in
    *.msgpack|*.h5|*.bin|*.ot|flax_model*|tf_model*|pytorch_model*) continue ;;
  esac
  mkdir -p "$dst/$(dirname "$f")"
  if [ ! -f "$dst/$f" ] || [ "$(stat -c %s "$dst/$f")" != "$size" ]; then
    nice -n 19 curl -sfL --retry 3 -o "$dst/$f" "https://huggingface.co/$repo/resolve/$rev/$f"
  fi
  actual_size=$(stat -c %s "$dst/$f")
  actual_sha=$(sha256sum "$dst/$f" | cut -d' ' -f1)
  echo "$actual_sha  $f" >> "$dst.sha256.tmp"
  size_ok=OK; [ "$actual_size" = "$size" ] || { size_ok=MISMATCH; ok=0; }
  if [ "$sha" = "-" ]; then
    echo "- $f: size expected $size / actual $actual_size ($size_ok); sha256 expected - / actual $actual_sha (no published hash, size only)" >> "$report"
  else
    sha_ok=OK; [ "$actual_sha" = "$sha" ] || { sha_ok=MISMATCH; ok=0; }
    echo "- $f: size expected $size / actual $actual_size ($size_ok); sha256 expected $sha / actual $actual_sha ($sha_ok)" >> "$report"
  fi
  total=$((total + actual_size)); nfiles=$((nfiles + 1))
done < <(jq -r '.siblings[] | [.rfilename, (.size|tostring), (.lfs.sha256 // "-")] | @tsv' "$listing")
mv "$dst.sha256.tmp" "$dst.sha256"
t1=$(date +%s)
{
  echo "Repo: $repo (https://huggingface.co/$repo)"
  echo "Revision: $rev"
  echo "License: $license"
  echo "Downloaded and verified: $start (UTC), nice -n 19 curl, $((t1 - t0)) s"
  echo "Target: $dst"
  if [ $ok = 1 ]; then
    echo "Result: $nfiles files, $total bytes total. Every size matched. Every LFS file's sha256 matched the Hugging Face API listing."
  else
    echo "Result: MISMATCH (see below). Do not use."
  fi
  echo "Note: small non-LFS files have no published hash in the listing, so only their size was checked."
  echo "Skipped: non-safetensors weight copies (pytorch .bin, TF .h5, Flax .msgpack)."
  echo
  echo "Per-file expected vs actual:"
  cat "$report"
} > "$dst.SOURCE"
rm -f "$listing" "$report"
cat "$dst.SOURCE" | head -8
[ $ok = 1 ]
