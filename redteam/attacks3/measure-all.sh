#!/bin/sh
# The smaller measurements of this session, as they were run (outputs in the .txt files named).
# Run in the sandbox from /work:  sh redteam/attacks3/measure-all.sh [bolt] tags|big-blob|zeros|packed-refs
set -u
BOLT=${1:-/tmp/t/release/bolt}
WHAT=${2:-tags}
A=/work/redteam/attacks3
M="python3 -I /work/redteam/attacks2/measure.py"
export BOLT_USER_NAME=T BOLT_USER_EMAIL=t@example.com BOLT_CONFIG_DIR=/tmp/ma-cfg
mkdir -p "$BOLT_CONFIG_DIR"
case "$WHAT" in
tags)  # RT3-9 and the tag-chain re-reads: tags.txt
    for c in "chain 10000" "chain 100000" "fat 100" "fat 480" "fat 500"; do
        set -- $c; d=$(mktemp -d); python3 -I $A/tags.py "$d" "$1" "$2"
        echo "$1, $2 tag refs: .git $(du -sk "$d/.git" | cut -f1) KB"
        $M 1200 "$d" "$BOLT" import | sed 's/^/  /'
        [ -d "$d/.bolt" ] && echo "  .bolt: $(du -sk "$d/.bolt" | cut -f1) KB"
        rm -rf "$d"
    done ;;
big-blob)  # RT3-8: big-blob.txt
    for mib in 128 256 512; do
        d=$(mktemp -d); python3 -I $A/big-blob.py "$d" "$mib"
        echo "one $mib MiB deltified file, .git $(du -sk "$d/.git" | cut -f1) KB:"
        $M 600 "$d" "$BOLT" import | sed 's/^/  /'
        rm -rf "$d"
    done ;;
zeros)  # RT3-3 end to end: inflate-sweep.txt, part 1
    for kib in 150000 204800 262144 262145 288000 307200; do
        d=$(mktemp -d); python3 -I $A/zeros.py "$d" "$kib" 6
        out=$(cd "$d" && "$BOLT" import 2>&1 >/dev/null | head -1 | sed 's/.*holds/holds/' | cut -c1-80)
        echo "KiB=$kib: ${out:-ok}"
        rm -rf "$d"
    done ;;
packed-refs)  # 7.5 million refs, within the limits: packed-refs.txt
    d=$(mktemp -d)
    python3 -I - "$d" <<'EOF'
import sys
sys.path.insert(0, "/work/redteam/attacks3")
from gitrepo import Repo, commit_bytes
repo = Repo(sys.argv[1])
c = repo.loose(b"commit", commit_bytes(repo.loose(b"tree", b""), [], b"one\n"))
repo.ref("refs/heads/main", c)
with open(sys.argv[1] + "/.git/packed-refs", "wb") as f:
    f.write(b"# pack-refs with: peeled fully-peeled sorted \n")
    for i in range(7_500_000):
        f.write(b"%s refs/pull/%07d/head\n" % (c.hex().encode(), i))
EOF
    echo "packed-refs $(stat -c %s "$d/.git/packed-refs") bytes; gzip: $(gzip -c "$d/.git/packed-refs" | wc -c) bytes"
    $M 600 "$d" "$BOLT" import
    rm -rf "$d" ;;
esac
