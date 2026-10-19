#!/bin/bash
B=$HOME/workspace/rig-speech-runs/bin/rig-listen-omega
D=$HOME/datasets/rig-speech; cd $HOME/workspace/rig-speech-runs/gpu2-out
echo "lock acquired $(date -u +%FT%TZ) load $(cat /proc/loadavg)"
$B gpu-selftest > gpu-selftest.json; echo selftest rc=$?
$B -q --backend omega --model $HOME/models/whisper-tiny-169d4a43 --out tiny-omega.json --receipt tiny-omega.receipt.json --machine GB10 $D/sample1.flac; echo tiny rc=$?
$B -q --backend omega --out turbo-omega-sample1.json --receipt turbo-omega-sample1.receipt.json --machine GB10 $D/sample1.flac; echo turbo rc=$?
RIG_DECODER_ON_GPU=1 $B -q --backend omega --out turbo-omega-decgpu-sample1.json --receipt turbo-omega-decgpu-sample1.receipt.json --machine GB10 $D/sample1.flac; echo turbo-decgpu rc=$?
for f in 1089-134686.mp4 1188-133604.flac 2961-960.flac; do
  n=${f%.*}
  $B -q --backend omega --out turbo-omega-$n.json --receipt turbo-omega-$n.receipt.json --machine GB10 $D/$f; echo $n rc=$?
done
echo "done $(date -u +%FT%TZ)"
