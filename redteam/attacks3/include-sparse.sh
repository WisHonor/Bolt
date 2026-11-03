#!/bin/sh
# RT3: `core.sparseCheckout = true` set through an `[include]`, which Git follows for that key.
# Bolt reads only .git/config itself, imports, and the first save records the files Git left out
# of the sparse folder as deleted. Run in the sandbox from /work:
#   sh redteam/attacks3/include-sparse.sh [bolt]
set -u
BOLT=${1:-/tmp/t/release/bolt}
export BOLT_USER_NAME=T BOLT_USER_EMAIL=t@example.com BOLT_CONFIG_DIR=/tmp/is-cfg
mkdir -p "$BOLT_CONFIG_DIR"
d=$(mktemp -d); cd "$d" || exit 1
python3 -I - <<'EOF'
import sys
sys.path.insert(0, "/work/redteam/attacks3")
from gitrepo import Repo, commit_bytes
repo = Repo(".")
blobs = {name: repo.loose(b"blob", name.encode() + b"\n") for name in ("a.txt", "b.txt", "c.txt")}
tree = repo.loose(b"tree", b"".join(b"100644 %s\0" % n.encode() + i for n, i in sorted(blobs.items())))
repo.ref("refs/heads/main", repo.loose(b"commit", commit_bytes(tree, [], b"three files\n")))
repo.file("config", b"[core]\n\trepositoryformatversion = 0\n[include]\n\tpath = sparse.inc\n")
repo.file("sparse.inc", b"[core]\n\tsparseCheckout = true\n")
repo.file("info/sparse-checkout", b"/a.txt\n")
open("a.txt", "w").write("a.txt\n")  # all a sparse checkout of /a.txt leaves in the folder
EOF
echo "\$ bolt import"; "$BOLT" import 2>&1
echo "\$ bolt save \"first\""; "$BOLT" save "first" 2>&1
echo "\$ bolt show"; "$BOLT" show 2>&1
cd / && rm -rf "$d"
