#!/bin/sh
# RT4 (session 1's "many packs" suspicion): every object lookup searches every pack in order.
# Empty packs pass every check and cost nothing, and objects/pack may hold 100,000 entries.
# Run in the sandbox from /work:  sh redteam/attacks4/many-packs.sh [bolt] [N] [P...]
set -u
BOLT=${1:-/tmp/t/release/bolt}
N=${2:-20000}
shift 2
COUNTS=${*:-"0 49999"}
export BOLT_USER_NAME=T BOLT_USER_EMAIL=t@example.com BOLT_CONFIG_DIR=/tmp/mp-cfg
mkdir -p "$BOLT_CONFIG_DIR"
for p in $COUNTS; do
    d=$(mktemp -d)
    python3 -I /work/redteam/attacks4/many-packs.py "$d" "$N" "$p"
    echo "N=$N files beside P=$p empty packs: .git $(du -sk "$d/.git" | cut -f1) KB on disk"
    echo "  tar -czf of .git: $(tar -C "$d" -czf - .git | wc -c) bytes"
    python3 -I /work/redteam/attacks2/measure.py 3600 "$d" "$BOLT" import | sed 's/^/  /'
    rm -rf "$d"
done
