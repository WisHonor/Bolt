#!/usr/bin/env bash
# Stops `bolt start --preset code` at each abort point in a folder that already has files and a
# .boltignore of the user's own, then runs `status`, `start` again, `verify` and `save`: the user's
# files must be untouched and the project must end up usable. Sandbox only.
set -u
BOLT=${BOLT:-/work/target/debug/bolt}
export BOLT_CONFIG_DIR=$(mktemp -d)
printf '[user]\nname = "T"\nemail = "t@example.invalid"\n' > "$BOLT_CONFIG_DIR/config.toml"
fresh() {
    p=$(mktemp -d); cd "$p"
    echo '{}' > package.json; echo hi > a.txt
    printf '\xef\xbb\xbfmine\r\n*.log\n# >>> Bolt preset: old\nx\n# <<< Bolt preset\ntail' > .boltignore
    head -c 2000000 /dev/urandom > big.bin
    cp .boltignore /tmp/b.ignore; cp big.bin /tmp/b.big
}
for n in $(seq 1 80); do
    fresh
    BOLT_FAULT_AT=$n $BOLT start --preset code > /tmp/out 2>&1; code=$?
    if [ $code -eq 0 ]; then echo "start: no stop at $n, done"; break; fi
    at=$(grep -o 'injected crash at [a-z:-]*' /tmp/out)
    $BOLT status > /tmp/st 2>&1; st=$?
    $BOLT start --preset code > /tmp/again 2>&1; again=$?
    $BOLT status > /tmp/st2 2>&1; st2=$?
    $BOLT verify > /tmp/v 2>&1; v=$?
    echo more >> a.txt
    $BOLT save "after" > /tmp/s 2>&1; s=$?
    files=ok
    cmp -s big.bin /tmp/b.big || files=BIG-CHANGED
    grep -q 'mine' .boltignore || files="$files IGNORE-LOST"
    grep -q 'tail' .boltignore || files="$files TAIL-LOST"
    grep -q 'hi' a.txt || files="$files A-LOST"
    echo "stop $n ($at): status $st, start again $again, status $st2, verify $v, save $s, files $files"
    if [ $st2 -ne 0 ] || [ $v -ne 0 ] || [ $s -ne 0 ] || [ "$files" != ok ]; then
        for f in /tmp/again /tmp/st2 /tmp/v /tmp/s; do tail -n 2 $f; done
    fi
done
