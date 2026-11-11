#!/usr/bin/env bash
# The abort points `bolt discard` of one edited file reaches, and the file after each. Sandbox only.
BOLT=${BOLT:-/work/target/debug/bolt}
export BOLT_CONFIG_DIR=$(mktemp -d)
printf '[user]\nname = "T"\nemail = "t@example.invalid"\n' > "$BOLT_CONFIG_DIR/config.toml"
for n in $(seq 1 20); do
    p=$(mktemp -d); cd "$p"
    $BOLT start --preset custom >/dev/null 2>&1; echo saved > a.txt; $BOLT save First >/dev/null 2>&1
    echo precious > a.txt
    BOLT_FAULT_AT=$n $BOLT discard > /tmp/o 2>&1 && { echo "$n: done"; break; }
    echo "$n: $(grep -o 'injected crash at [a-z:-]*' /tmp/o) a.txt=$(cat a.txt) journal=$(ls .bolt/state)"
done
