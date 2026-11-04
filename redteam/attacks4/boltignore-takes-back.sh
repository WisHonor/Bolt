#!/bin/sh
# RT4: a repository tracks a placeholder .env, lists .env and *.pem in .gitignore, and tracks a
# .boltignore that takes both back. Git ignores them; Bolt's save takes the user's real .env and
# a new private key without a word.
# Run in the sandbox from /work:  sh redteam/attacks4/boltignore-takes-back.sh [bolt]
set -u
BOLT=${1:-/tmp/t/release/bolt}
export BOLT_USER_NAME=T BOLT_USER_EMAIL=t@example.com BOLT_CONFIG_DIR=/tmp/bi-cfg
mkdir -p "$BOLT_CONFIG_DIR"
d=$(mktemp -d)
python3 -I - "$d" <<'EOF'
import sys
sys.path.insert(0, "/work/redteam/attacks3")
from gitrepo import Repo, oid, commit_bytes
import os
root = sys.argv[1]
repo = Repo(root)
files = [(b".boltignore", b"!.env\n!*.pem\n"), (b".env", b"API_KEY=put-yours-here\n"),
         (b".gitignore", b".env\n*.pem\n"), (b"app.py", b"print('hi')\n")]
entries = b""
for name, data in files:
    entries += b"100644 " + name + b"\0" + repo.loose(b"blob", data)
    with open(os.path.join(root, name.decode()), "wb") as f:
        f.write(data)
tree = repo.loose(b"tree", entries)
repo.ref("refs/heads/main", repo.loose(b"commit", commit_bytes(tree, [], b"Start\n")))
EOF
cd "$d"
echo '$ bolt import'; "$BOLT" import 2>&1
printf 'API_KEY=sk_live_REAL_SECRET\n' > .env
printf -- '-----BEGIN PRIVATE KEY-----\nMIIE...\n' > deploy.pem
printf "print('hello')\n" > app.py
echo '$ bolt save "Say hello"'; "$BOLT" save "Say hello" 2>&1
echo '$ bolt show'; "$BOLT" show 2>&1
rm -rf "$d"
