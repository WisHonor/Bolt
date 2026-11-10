#!/usr/bin/env bash
# RT13 combination: many small packs (one a save, never tidied) under a deep tree, so the walk's
# store and the read-ahead's second handle both search every pack for every listing. `status`,
# `save`, `goto` and `changes`, timed, under `ulimit -n 256`, this build against main's.
# Run in the sandbox only:  redteam/attacks12/run.sh script many_packs_deep.sh [saves] [depth]
set -u
NEW=/work/target/release/bolt
OLD=/work/_main/target/release/bolt
SAVES=${1:-1000}
DEPTH=${2:-200}
work=$(mktemp -d)
export BOLT_USER_NAME=r BOLT_USER_EMAIL=r@x BOLT_CONFIG_DIR=$work/cfg HOME=$work/cfg
mkdir -p "$work/p" "$work/cfg"
cd "$work/p"
$NEW start >/dev/null
d=.
for i in $(seq 1 "$DEPTH"); do d=$d/d; mkdir -p "$d"; echo "$i" >"$d/f"; done
$NEW save base >/dev/null
for s in $(seq 1 "$SAVES"); do
    lvl=$(( (s * 37) % DEPTH + 1 ))
    p=.; for i in $(seq 1 $lvl); do p=$p/d; done
    echo "s$s" >"$p/f"
    $NEW save "s$s" >/dev/null || { echo "save $s failed"; break; }
done
echo "packs: $(ls .bolt/store/packs 2>/dev/null | wc -l) $(ls .bolt 2>/dev/null | tr '\n' ' ')"
first=$($NEW history | tail -2 | head -1 | awk '{print $1}')
cd /
for who in new old; do
    B=$NEW; [ $who = old ] && B=$OLD
    rm -rf "$work/$who"; cp -a "$work/p" "$work/$who"; cd "$work/$who"
    export BOLT_CONFIG_DIR=$work/cfg-$who; mkdir -p "$BOLT_CONFIG_DIR"
    t() {
        local s=$(date +%s.%N)
        out=$(sh -c 'ulimit -n 256; exec "$@"' sh "$B" "$@" 2>&1); c=$?
        local e=$(date +%s.%N)
        printf '%s %-24s %6.2fs exit %s %s\n' "$who" "$*" "$(awk "BEGIN{print $e - $s}")" "$c" "$(echo "$out" | head -2 | tr '\n' '|' | cut -c1-120)"
    }
    t status
    t status
    p=.; for i in $(seq 1 "$DEPTH"); do p=$p/d; done; echo deepest >"$p/f"
    t status
    t changes
    t save deepest
    t goto "$first"
    t status
    t goto main
    cd /
done
rm -rf "$work"
