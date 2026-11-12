#!/usr/bin/env bash
# RT14: --json on hostile names. Every command's JSON must parse, and each name must come back
# as the bytes (or the escaped form) it was. Sandbox only.
set -u
export PATH=/work/target/release:$PATH
W=$(mktemp -d)
cd "$W"
bolt start --preset custom >/dev/null
python3 - <<'EOF'
import os
names = [
    'quote"brace}{', 'back\\slash', 'ctl\x01\x07\x1b[31mred', 'tab\tnl\nlf', 'del\x7f',
    '{"bolt_json":1,"x":"', ' line sep', '﻿bom', 'é', 'long' + 'L' * 250,
    'bidi‮exe.txt', 'nul-ish\\u0000', '😀emoji', '퟿',
]
for n in names:
    with open(n, 'w') as f:
        f.write('x')
os.makedirs('deep/' * 1, exist_ok=True)
EOF
# A name that isn't UTF-8, which Bolt can't save but must show.
printf 'x' > "$(printf 'bad\377\376name')"
check() {
    local label=$1; shift
    out=$("$@" 2>/dev/null); rc=$?
    if printf '%s' "$out" | python3 -c 'import json,sys; json.loads(sys.stdin.read())' 2>/dev/null; then
        echo "$label rc=$rc json=ok bytes=${#out}"
    else
        echo "$label rc=$rc json=BROKEN"; printf '%s' "$out" | head -c 600; echo
    fi
}
check status bolt status --json
check save bolt save "first" --json --yes
rm -f "$(printf 'bad\377\376name')"
bolt save "first" --json --yes; echo
check status2 bolt status --json
check show bolt show --json
check changes-save bolt changes --save main --json
check history bolt history --json
for f in *; do [ -f "$f" ] && printf 'y' >> "$f" 2>/dev/null; done
check changes bolt changes --json
check line bolt line --json
check verify bolt verify --json
check tidy bolt tidy --json
check undo-list bolt undo --list --json
# The names must come back as they are.
bolt show --json | python3 -c '
import json,sys,os
d=json.load(sys.stdin)
s=json.dumps(d)
want=[n for n in os.listdir(".") if not n.startswith(".")]
print("names on disk", len(want))
miss=[n for n in want if json.dumps(n)[1:-1] not in s]
print("names not found in show --json:", [repr(m) for m in miss][:5])
'
rm -rf "$W"
