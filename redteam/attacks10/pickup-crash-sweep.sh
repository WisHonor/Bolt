#!/usr/bin/env bash
# A discard stopped after writing the saved file over unsaved work, then `head` lost; then
# `bolt undo --to <that entry>` stopped at each abort point, then run again, then status, verify
# and tidy. The unsaved work must stay in the store (its marker found in a pack) and every later
# command must finish or refuse cleanly. Sandbox only.
set -u
BOLT=${BOLT:-/work/target/debug/bolt}
export BOLT_CONFIG_DIR=$(mktemp -d)
printf '[user]\nname = "T"\nemail = "t@example.invalid"\n' > "$BOLT_CONFIG_DIR/config.toml"
MARK=precious-work-7f3a9c
# The abort point of discard that comes after the file is written.
fresh() {
    p=$(mktemp -d); cd "$p"
    $BOLT start --preset custom >/dev/null 2>&1; echo saved > a.txt; $BOLT save First >/dev/null 2>&1
    echo "$MARK" > a.txt
}
at=""
for n in $(seq 1 20); do
    fresh
    BOLT_FAULT_AT=$n $BOLT discard > /tmp/o 2>&1 && break
    grep -q 'switch:changed' /tmp/o && { at=$n; break; }
done
[ -z "$at" ] && { echo "no switch:changed point found"; exit 1; }
echo "discard stops at switch:changed at point $at"
for m in $(seq 1 40); do
    fresh
    BOLT_FAULT_AT=$at $BOLT discard >/dev/null 2>&1
    entry=$(cat .bolt/state/head | tr -d '\n')
    rm .bolt/state/head
    BOLT_FAULT_AT=$m $BOLT undo --to "$entry" > /tmp/u1 2>&1; u1=$?
    stop=$(grep -o 'injected crash at [a-z:-]*' /tmp/u1)
    $BOLT status > /tmp/st 2>&1; st=$?
    $BOLT undo --to "$entry" > /tmp/u2 2>&1; u2=$?
    $BOLT status > /tmp/st2 2>&1; st2=$?
    $BOLT verify > /tmp/v 2>&1; v=$?
    $BOLT tidy > /tmp/t 2>&1; t=$?
    kept=no; grep -rqa "$MARK" .bolt/store/packs && kept=yes
    [ "$(cat a.txt)" = "$MARK" ] && kept="$kept (in a.txt)"
    echo "stop $m ($stop): undo $u1, status $st, undo again $u2, status $st2, verify $v, tidy $t, work kept $kept"
    if [ $st2 -ne 0 ] || [ $v -ne 0 ] || [ $t -ne 0 ] || [ "${kept%% *}" = no ]; then
        for f in /tmp/u2 /tmp/st2 /tmp/v /tmp/t; do tail -n 3 "$f"; done
    fi
    [ -z "$stop" ] && break
done
