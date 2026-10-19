#!/bin/bash
B=./rig-listen-cpu
D=$HOME/datasets/rig-speech
for f in 1089-134686.mp4 1188-133604.flac 2961-960.flac; do
  n=${f%.*}
  echo "start $f $(date -u +%FT%TZ) load $(cut -d\" \" -f1-3 /proc/loadavg)"
  nice -n 10 $B -q --backend cpu --out turbo-cpu-$n.json --receipt turbo-cpu-$n.receipt.json --machine GB10 $D/$f
  $B wer $D/$n.ref.txt turbo-cpu-$n.json > turbo-cpu-$n.wer.json
  cat turbo-cpu-$n.wer.json
done
echo "done $(date -u +%FT%TZ)"
