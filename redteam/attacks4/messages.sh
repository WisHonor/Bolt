#!/bin/sh
# RT4: commits Git stores as deltas, Bolt stores whole. N commits share one random 1 MB message;
# Git's pack holds it once, and each commit is a ~80-byte delta on the first. No merges, so the
# limit on parents (RT3-2's fix) doesn't apply.
# Run in the sandbox from /work:  sh redteam/attacks4/messages.sh [bolt] [N...]
set -u
BOLT=${1:-/tmp/t/release/bolt}
shift
SIZES=${*:-"500 2000"}
export BOLT_USER_NAME=T BOLT_USER_EMAIL=t@example.com BOLT_CONFIG_DIR=/tmp/msg-cfg
mkdir -p "$BOLT_CONFIG_DIR"
for n in $SIZES; do
    d=$(mktemp -d)
    python3 -I /work/redteam/attacks4/messages.py "$d" "$n"
    echo "N=$n commits sharing one 1 MB message: .git $(du -sk "$d/.git" | cut -f1) KB on disk"
    echo "  tar -czf of .git: $(tar -C "$d" -czf - .git | wc -c) bytes"
    python3 -I /work/redteam/attacks2/measure.py 1800 "$d" "$BOLT" import | sed 's/^/  /'
    [ -d "$d/.bolt" ] && echo "  .bolt afterwards: $(du -sk "$d/.bolt" | cut -f1) KB"
    rm -rf "$d"
done
