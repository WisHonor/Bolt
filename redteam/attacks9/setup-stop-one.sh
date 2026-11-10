#!/usr/bin/env bash
# One stop of `bolt setup --preset code --yes` at abort point $1, every output shown. Sandbox only.
set -u
BOLT=${BOLT:-/work/target/debug/bolt}
export BOLT_CONFIG_DIR=$(mktemp -d)
printf '[user]\nname = "T"\nemail = "t@example.invalid"\n' > "$BOLT_CONFIG_DIR/config.toml"
p=$(mktemp -d); cd "$p"
echo '{}' > package.json
$BOLT start --preset custom >/dev/null 2>&1
printf 'preset = "mine"\nchunk-files-over = "2 MiB"\n' > bolt.toml
echo hi > a.txt
$BOLT save "with rules" >/dev/null 2>&1
printf 'preset = "mine"\n# unsaved edit\n' > bolt.toml
show() { echo "   bolt.toml: $(head -c 50 bolt.toml | tr '\n' '|')"; }
BOLT_FAULT_AT=$1 $BOLT setup --preset code --yes 2>&1 | tail -2; show
for c in "${@:2}"; do echo "== bolt $c"; $BOLT $c 2>&1 | tail -6; show; done
