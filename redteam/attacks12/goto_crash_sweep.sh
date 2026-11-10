#!/usr/bin/env bash
# Crash `bolt goto` at every fault point while it writes a 40 MiB file and some small ones, let
# the next command finish it, then check that what `status` says matches the folder's bytes.
# Run in the sandbox only:  redteam/attacks12/run.sh script goto_crash_sweep.sh [bolt] [from] [to]
set -u
B=${1:-/work/target/release/bolt}
FROM=${2:-1}
TO=${3:-400}
work=$(mktemp -d)
export BOLT_USER_NAME=r BOLT_USER_EMAIL=r@x
head -c 41943040 /dev/urandom >"$work/big1"
cp "$work/big1" "$work/big2"
# Same size, a few bytes changed in the middle and near the end.
printf 'XXXXXXXX' | dd of="$work/big2" bs=1 seek=20000000 conv=notrunc 2>/dev/null
printf 'YYYYYYYY' | dd of="$work/big2" bs=1 seek=41900000 conv=notrunc 2>/dev/null

fresh() {
    rm -rf "$work/p" "$work/cfg"
    mkdir -p "$work/p" "$work/cfg"
    export BOLT_CONFIG_DIR=$work/cfg HOME=$work/cfg
    cd "$work/p"
    $B start >/dev/null
    cp "$work/big1" big
    mkdir -p sub
    for f in a b c d; do echo "one $f" >"sub/$f"; done
    echo keep >k
    $B save s1 >/dev/null
    cp "$work/big2" big
    echo "two a" >sub/a
    rm sub/b
    echo "new e" >sub/e
    echo "two c" >sub/c
    $B save s2 >/dev/null
    $B status >/dev/null
}

sum() { (cd "$work/p" && find . -path ./.bolt -prune -o -type f -print | sort | xargs sha256sum); }

fresh
id1=$($B history | awk '$NF=="s1"{print $1}')
$B goto "$id1" >/dev/null || echo "goto $id1 failed"
s1=$(sum)
$B goto main >/dev/null || echo "goto main failed"
s2=$(sum)
[ "$s1" != "$s2" ] || echo "s1 and s2 sum the same: the script is broken"
cd /
for n in $(seq "$FROM" "$TO"); do
    fresh
    id1=$($B history | awk '$NF=="s1"{print $1}')
    BOLT_FAULT_AT=$n $B goto "$id1" >"$work/out" 2>&1
    code=$?
    if [ $code -eq 0 ]; then
        echo "n=$n goto finished without a crash: sweep done"
        break
    fi
    at=$(grep -o 'injected crash at .*' "$work/out" | head -1)
    $B status >"$work/st1" 2>&1
    c1=$?
    now=$(sum)
    which=other
    [ "$now" = "$s1" ] && which=s1
    [ "$now" = "$s2" ] && which=s2
    said=$(head -c 160 "$work/st1" | tr '\n' '|')
    flag=""
    if grep -q "Nothing changed" "$work/st1"; then
        pos=$($B history 2>/dev/null | head -1 | awk '{print $NF}')
        if [ "$which" = other ] || [ "$pos" != "$which" ]; then flag="  <<< status says nothing changed, folder is $which, history top $pos"; fi
    fi
    $B changes >"$work/v" 2>&1 || flag="$flag  <<< changes failed: $(head -c 200 "$work/v" | tr '\n' '|')"
    $B goto main >"$work/v" 2>&1 || flag="$flag  <<< goto main: $(head -c 200 "$work/v" | tr '\n' '|')"
    [ "$(sum)" = "$s2" ] || flag="$flag  <<< after goto main the folder is not s2"
    left=$(ls .bolt/tmp 2>/dev/null | wc -l)
    echo "n=$n [$at] status=$c1 folder=$which tmp=$left said: $said$flag"
    cd /
done
rm -rf "$work"
