#!/usr/bin/env bash
# RT12-1 follow-up: after the amend aborts under the 1.5 GB cap, is the project as it was?
# Run in the sandbox only:  redteam/attacks11/run.sh script amend_after_abort.sh
set -u
B=/work/target/release/bolt
base=$(mktemp -d); export BOLT_CONFIG_DIR=$base/cfg HOME=$base/cfg BOLT_USER_NAME=r BOLT_USER_EMAIL=r@x
mkdir -p $base/cfg $base/p; cd $base/p
git init -q -b main .
python3 - <<'EOF' | GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@b git fast-import --quiet
import sys
out = sys.stdout.buffer
for i in range(1000):
    d = b"v%d\n" % i
    out.write(b"commit refs/heads/main\nmark :%d\ncommitter a <a@b> %d +0000\ndata 2\nc\n" % (i + 1, 1700000000 + i))
    if i:
        out.write(b"from :%d\n" % i)
    out.write(b"M 100644 inline f\ndata %d\n%s\n" % (len(d), d))
EOF
p=$(git rev-parse main~1)
for j in $(seq -w 1 10000); do echo "create refs/heads/b$j $p"; done | git update-ref --stdin
git checkout -q -f main
$B import | tail -1
before=$($B history | head -1)
echo more >> f
sh -c 'ulimit -v 1500000; exec "$0" save --amend m' $B 2>&1 | head -2; echo "exit ${PIPESTATUS[0]}"
echo "history before: $before"
echo "history after:  $($B history | head -1)"
$B status
ls .bolt/tmp | wc -l
cd /; rm -rf $base
