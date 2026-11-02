#!/bin/sh
# Does a deeply nested project .bolt/config.toml (under the 1 MiB cap) crash `bolt save`?
# Usage (in the sandbox): sh redteam/attacks2/toml-depth.sh [path-to-bolt]
BOLT=${1:-/tmp/t/debug/bolt}
export BOLT_USER_NAME=T BOLT_USER_EMAIL=t@example.com BOLT_CONFIG_DIR=/tmp/toml-depth-cfg
mkdir -p "$BOLT_CONFIG_DIR"
for shape in array inline-table dotted; do
  d=$(mktemp -d); cd "$d" || exit 1
  "$BOLT" start >/dev/null || exit 1
  echo hi > a.txt
  case $shape in
    array)        python3 -c "import sys; sys.stdout.write('x = ' + '['*500000)" ;;
    inline-table) python3 -c "import sys; sys.stdout.write('x = ' + '{a='*250000)" ;;
    dotted)       python3 -c "import sys; sys.stdout.write('a.'*450000 + 'b = 1\n')" ;;
  esac > .bolt/config.toml
  ls -l .bolt/config.toml | awk '{print "config bytes:", $5}'
  "$BOLT" save "probe" >/dev/null 2>err.txt; code=$?
  echo "$shape: exit $code; stderr: $(head -c 300 err.txt)"
  cd / && rm -rf "$d"
done
