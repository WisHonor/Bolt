#!/bin/sh
# RT2-2 at the format's maximum: 100,000 lines (MAX_LINES) over a 300-save chain, release build.
#   sh redteam/attacks2/resolve-max-lines.sh
set -u
export CARGO_TARGET_DIR=/tmp/t BOLT_USER_NAME=Test BOLT_USER_EMAIL=t@example.com
export BOLT_CONFIG_DIR=/tmp/rm-cfg
mkdir -p "$BOLT_CONFIG_DIR"
d=$(mktemp -d)
RT2_DIR=$d RT2_KIND=lines RT2_N=300 RT2_LINES=99999 \
  cargo test -q -p bolt-vcs --test redteam2 -- --ignored plant_for_script --nocapture 2>&1 | grep -o 'packs: [0-9]* bytes'
echo "on disk: $(du -sk "$d/.bolt" | cut -f1) KB"
python3 -I redteam/attacks2/measure.py 1200 "$d" /tmp/t/release/bolt history
python3 -I redteam/attacks2/measure.py 1200 "$d" /tmp/t/release/bolt show 0123abcd
rm -rf "$d"
