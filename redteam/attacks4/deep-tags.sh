#!/bin/sh
# RT4: lightweight tags on a commit 4,096 deltas deep. Pass 1 reads each tag's target's header,
# which walks the whole delta chain every time, outside the base cache and the work allowance:
# 100,000 tags make it read 409,600,000 entry headers.
# Run in the sandbox from /work:  sh redteam/attacks4/deep-tags.sh [bolt] [T...]
set -u
BOLT=${1:-/tmp/t/release/bolt}
shift
SIZES=${*:-"1000 10000 100000"}
export BOLT_USER_NAME=T BOLT_USER_EMAIL=t@example.com BOLT_CONFIG_DIR=/tmp/dt-cfg
mkdir -p "$BOLT_CONFIG_DIR"
for t in $SIZES; do
    d=$(mktemp -d)
    python3 -I /work/redteam/attacks4/deep-tags.py "$d" "$t"
    echo "T=$t tags on a commit 4,096 deltas deep: .git $(du -sk "$d/.git" | cut -f1) KB on disk"
    echo "  tar -czf of .git: $(tar -C "$d" -czf - .git | wc -c) bytes"
    python3 -I /work/redteam/attacks2/measure.py 3600 "$d" "$BOLT" import | sed 's/^/  /'
    rm -rf "$d"
done
# The same 100,000 tags on a commit stored whole, for comparison.
d=$(mktemp -d)
python3 -I /work/redteam/attacks4/deep-tags.py "$d" 100000 0
echo "T=100000 tags on a commit stored whole: .git $(du -sk "$d/.git" | cut -f1) KB on disk"
python3 -I /work/redteam/attacks2/measure.py 3600 "$d" "$BOLT" import | sed 's/^/  /'
rm -rf "$d"
