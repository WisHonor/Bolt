#!/bin/sh
# RT3: a chain of 1,024-parent merges, each a ~60-byte delta in Git's pack. Pass 1 of the import
# holds every merge's 1,024 parents twice on its stack, and every save stores its parents' 32-byte
# IDs whole, so memory and the new .bolt grow ~67 KB and ~33 KB a commit.
# Run in the sandbox from /work:  sh redteam/attacks3/octopus.sh [bolt] [N...]
set -u
BOLT=${1:-/tmp/t/release/bolt}
shift
SIZES=${*:-"1000 5000"}
export BOLT_USER_NAME=T BOLT_USER_EMAIL=t@example.com BOLT_CONFIG_DIR=/tmp/oc-cfg
mkdir -p "$BOLT_CONFIG_DIR"
for n in $SIZES; do
    d=$(mktemp -d)
    python3 -I /work/redteam/attacks3/octopus.py "$d" "$n"
    echo "N=$n merges of 1,024 parents: .git $(du -sk "$d/.git" | cut -f1) KB on disk"
    python3 -I /work/redteam/attacks2/measure.py 900 "$d" "$BOLT" import | sed 's/^/  /'
    [ -d "$d/.bolt" ] && echo "  .bolt afterwards: $(du -sk "$d/.bolt" | cut -f1) KB"
    rm -rf "$d"
done
