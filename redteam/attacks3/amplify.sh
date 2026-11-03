#!/bin/sh
# RT3: delta-base amplification. N small commits, each a delta on one huge base Bolt's 256 MiB
# base cache can't hold: every read of every commit inflates the whole base again (pass 1 and
# pass 2 each read every commit). `depth` > 1 puts the base under a chain of deltas that each
# rebuild it, so each read redoes the whole chain.
# Run in the sandbox from /work:  sh redteam/attacks3/amplify.sh [bolt] "N:MiB:depth" ...
set -u
BOLT=${1:-/tmp/t/release/bolt}
shift
CASES=${*:-"4:300:1 16:300:1"}
export BOLT_USER_NAME=T BOLT_USER_EMAIL=t@example.com BOLT_CONFIG_DIR=/tmp/am-cfg
mkdir -p "$BOLT_CONFIG_DIR"
for c in $CASES; do
    n=${c%%:*}; rest=${c#*:}; mib=${rest%%:*}; depth=${rest#*:}
    d=$(mktemp -d)
    python3 -I /work/redteam/attacks3/amplify.py "$d" "$n" "$mib" "$depth"
    echo "N=$n commits on a $mib MiB base under $depth delta(s): .git $(du -sk "$d/.git" | cut -f1) KB on disk"
    python3 -I /work/redteam/attacks2/measure.py 3600 "$d" "$BOLT" import | sed 's/^/  /'
    rm -rf "$d"
done
