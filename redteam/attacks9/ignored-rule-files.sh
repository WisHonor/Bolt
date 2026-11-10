#!/usr/bin/env bash
# The setup step when .boltignore leaves out the rule files themselves. Sandbox only.
set -u
BOLT=${BOLT:-/work/target/debug/bolt}
export BOLT_CONFIG_DIR=$(mktemp -d)
printf '[user]\nname = "T"\nemail = "t@example.invalid"\n' > "$BOLT_CONFIG_DIR/config.toml"
for ignored in "bolt.toml" ".boltignore" "bolt.toml\n.boltignore" "*.toml" "/*"; do
    p=$(mktemp -d); cd "$p"
    echo "== .boltignore: $ignored"
    $BOLT start --preset custom >/dev/null 2>&1
    printf "$ignored\n" > .boltignore
    printf 'preset = "mine"\n# my own rules\n' > bolt.toml
    echo hi > a.txt
    $BOLT save "with rules" >/dev/null 2>&1
    printf 'preset = "mine"\n# my own rules, edited, unsaved\n' > bolt.toml
    cp bolt.toml /tmp/before.toml; cp .boltignore /tmp/before.ignore
    $BOLT setup --preset code --yes > /tmp/out 2>&1; echo "setup: $?"; tail -2 /tmp/out
    echo "-- bolt.toml now: $(head -c 60 bolt.toml | tr '\n' '|')"
    $BOLT undo > /tmp/out 2>&1; echo "undo: $?"; tail -2 /tmp/out
    cmp -s bolt.toml /tmp/before.toml && echo "bolt.toml back" || echo "BOLT.TOML NOT BACK: $(head -c 80 bolt.toml | tr '\n' '|')"
    cmp -s .boltignore /tmp/before.ignore && echo ".boltignore back" || echo ".BOLTIGNORE NOT BACK: $(head -c 80 .boltignore 2>&1| tr '\n' '|')"
    $BOLT redo > /tmp/out 2>&1; echo "redo: $?"; tail -1 /tmp/out
    $BOLT undo > /dev/null 2>&1
    cmp -s bolt.toml /tmp/before.toml && echo "bolt.toml back after redo+undo" || echo "BOLT.TOML NOT BACK after redo+undo"
done
