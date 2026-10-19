#!/usr/bin/env bash
# Build a long-form test clip from one LibriSpeech chapter (public, CC BY 4.0):
# all utterances of the chapter joined in order into <out>/<spk>-<chap>.flac (16 kHz mono),
# the same audio as <out>/<spk>-<chap>.mp4 (black 640x360 video + AAC 48 kHz stereo, to test
# the video path: container, resampling, downmix), and the joined reference text in
# <out>/<spk>-<chap>.ref.txt.
#
# ffmpeg is used here only to make test inputs. rig-listen itself never calls it.
#
# Usage: make-librispeech-fixture.sh <LibriSpeech/test-clean dir> <speaker> <chapter> <out dir>
set -euo pipefail
root="$1"; spk="$2"; chap="$3"; out="$4"
dir="$root/$spk/$chap"
mkdir -p "$out"
name="$spk-$chap"
list="$(mktemp)"
for f in $(ls "$dir"/*.flac | sort); do echo "file '$f'" >> "$list"; done
ffmpeg -hide_banner -loglevel error -y -f concat -safe 0 -i "$list" -ac 1 -ar 16000 -c:a flac "$out/$name.flac"
ffmpeg -hide_banner -loglevel error -y -f lavfi -i color=c=black:s=640x360:r=25 -i "$out/$name.flac" \
  -shortest -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac -ar 48000 -ac 2 -b:a 128k "$out/$name.mp4"
sort "$dir/$name.trans.txt" | cut -d' ' -f2- > "$out/$name.ref.txt"
rm -f "$list"
echo "$out/$name.flac $out/$name.mp4 $out/$name.ref.txt ($(wc -w < "$out/$name.ref.txt") words)"
