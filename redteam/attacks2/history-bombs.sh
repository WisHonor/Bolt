#!/bin/sh
# RT2-1 and RT2-2, measured with a RELEASE build and no memory cap (the sandbox's own 4 GB
# limit is the only one). Run in the sandbox from /work:
#   sh redteam/attacks2/history-bombs.sh
set -u
BOLT=/tmp/t/release/bolt
export CARGO_TARGET_DIR=/tmp/t BOLT_USER_NAME=Test BOLT_USER_EMAIL=t@example.com
export BOLT_CONFIG_DIR=/tmp/hb-cfg
mkdir -p "$BOLT_CONFIG_DIR"
M="python3 -I redteam/attacks2/measure.py"
plant() { # kind n lines
  d=$(mktemp -d)
  RT2_DIR=$d RT2_KIND=$1 RT2_N=$2 RT2_LINES=${3:-0} \
    cargo test -q -p bolt-vcs --test redteam2 -- --ignored plant_for_script --nocapture 2>&1 \
    | grep -o 'packs: [0-9]* bytes' >&2
  echo "$d"
}
echo "== RT2-1: N saves with 1 MiB messages; memory grows with N, the packs barely do"
for n in 500 1000 6000; do
  d=$(plant history $n)
  echo "N=$n ($(du -sk "$d/.bolt" | cut -f1) KB on disk)"
  $M 300 "$d" $BOLT history
  echo hi > "$d/mine.txt"; $M 300 "$d" $BOLT save mine
  rm -rf "$d"
done
echo "== RT2-2: 300 small saves; time of 'show <8 hex digits>' grows with the number of lines"
for lines in 1000 10000; do
  d=$(plant lines 300 $lines)
  $M 600 "$d" $BOLT show 0123abcd
  rm -rf "$d"
done
