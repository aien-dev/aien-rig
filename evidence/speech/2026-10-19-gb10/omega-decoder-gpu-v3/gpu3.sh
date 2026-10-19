#!/bin/bash
B=$HOME/workspace/rig-speech-runs/bin/rig-listen-omega-v3
D=$HOME/datasets/rig-speech; cd $HOME/workspace/rig-speech-runs/gpu3-out
echo "lock acquired $(date -u +%FT%TZ) load $(cat /proc/loadavg)"
$B gpu-selftest > gpu-selftest.json; echo selftest rc=$?
for f in 1089-134686.mp4 1188-133604.flac 2961-960.flac; do
  n=${f%.*}
  $B -q --backend omega --out turbo-omega-$n.json --receipt turbo-omega-$n.receipt.json --machine GB10 $D/$f; echo $n rc=$?
done
echo "done $(date -u +%FT%TZ)"
