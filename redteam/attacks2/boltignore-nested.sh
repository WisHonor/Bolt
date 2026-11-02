#!/bin/sh
# RT2-6: the scan keeps the compiled rules of every folder on its way down, so nested hostile
# .boltignore files (each under the 1 MiB cap) add up. `bolt save` in an unpacked project:
#   sh redteam/attacks2/boltignore-nested.sh [bolt]
set -u
BOLT=${1:-/tmp/t/release/bolt}
export BOLT_USER_NAME=T BOLT_USER_EMAIL=t@example.com BOLT_CONFIG_DIR=/tmp/bn-cfg
mkdir -p "$BOLT_CONFIG_DIR"
pat=$(mktemp)
python3 -I -c "
import sys
out, i = [], 0
while sum(map(len, out)) < (1 << 20) - 300:
    out.append('[a-zA-Z0-9_]' * 20 + '%d\n' % i); i += 1
sys.stdout.write(''.join(out))" > "$pat"
for depth in ${DEPTHS:-1 4 8 16 32}; do
  d=$(mktemp -d); cd "$d" || exit 1
  "$BOLT" start >/dev/null
  p=$d; i=0
  while [ $i -lt $depth ]; do cp "$pat" "$p/.boltignore"; echo hi > "$p/f.txt"; p=$p/d; mkdir "$p"; i=$((i+1)); done
  tar -czf /tmp/bn.tgz --exclude=.bolt -C "$d" .
  printf 'depth %2d (%s KB as .tar.gz): ' $depth $(( $(stat -c %s /tmp/bn.tgz) / 1024 ))
  python3 -I /work/redteam/attacks2/measure.py 300 "$d" "$BOLT" save x
  cd / && rm -rf "$d" /tmp/bn.tgz
done
rm -f "$pat"
