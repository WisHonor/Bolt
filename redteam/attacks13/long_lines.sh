#!/usr/bin/env bash
# RT14: one long hostile .gitignore line, at two sizes: `status` time must grow linearly.
# Sandbox only.
set -u
export PATH=/work/target/release:$PATH
for shape in range class open-class escapes stars dirs; do
    for kb in 100 900; do
        W=$(mktemp -d); cd "$W"
        for i in $(seq 1 50); do printf x > "f$i.txt"; done
        python3 - "$shape" "$kb" > .gitignore <<'PY'
import sys
shape, kb = sys.argv[1], int(sys.argv[2]) * 1000
unit = {"range": "a-", "class": "[:alpha:]", "open-class": "[:", "escapes": "\\]",
        "stars": "*a", "dirs": "**/"}[shape]
body = unit * (kb // len(unit))
line = {"range": "[" + body + "]q", "class": "[" + body + "]q", "open-class": "[" + body + "]q",
        "escapes": "[" + body + "]q", "stars": body + "q", "dirs": body + "q"}[shape]
sys.stdout.write(line + "\n")
PY
        bolt start --preset custom >/dev/null 2>&1
        python3 - "$shape" "$kb" <<'PY'
import subprocess, time, sys
s = time.time()
r = subprocess.run(["bolt", "status"], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=600)
print(f"{sys.argv[1]:<10} {sys.argv[2]:>4} KB  bolt status {time.time() - s:6.2f}s rc={r.returncode}")
PY
        cd /; rm -rf "$W"
    done
done
