#!/usr/bin/env bash
# RT14: what a large hostile .gitignore costs Bolt's status against Git's, per pattern shape.
# Sandbox only.
set -u
export PATH=/work/target/release:$PATH
FILES=${FILES:-2000}
run() {
    local shape=$1 line=$2
    W=$(mktemp -d); cd "$W"
    mkdir -p d
    for i in $(seq 1 "$FILES"); do printf x > "d/f$i.txt"; done
    # About 1 MiB of lines that match nothing here.
    python3 -c "
import sys
line=sys.argv[1]
n=900_000//(len(line)+6)
sys.stdout.write(''.join(line.replace('#', str(i)) + '\n' for i in range(n)))" "$line" > .gitignore
    bolt start --preset custom >/dev/null 2>&1; git init -q . >/dev/null 2>&1

    python3 - "$shape" <<'PY'
import subprocess, time, resource, sys
def t(cmd):
    s = time.time()
    r = subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=600)
    return time.time() - s, r.returncode, r.stderr.decode(errors="replace")[:120]
g, _, _ = t(["git", "status", "--porcelain", "-uall"])
b, rc, err = t(["bolt", "status"])
peak = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
lines = sum(1 for _ in open(".gitignore", "rb"))
print(f"{sys.argv[1]:<10} lines={lines:<7} git={g:6.2f}s bolt={b:6.2f}s rc={rc} peak={peak}KiB {err!r}")
PY
    cd /; rm -rf "$W"
}
run plain 'zz#'
run star 'zz#*q'
run glob '[!a-z]#*[xy]?'
run dirs '**/z#/**/q*'
run classes '[[:alpha:][:digit:]]#[[:space:]]*'
run anchored 'd/z#*/q'
