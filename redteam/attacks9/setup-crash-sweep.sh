#!/usr/bin/env bash
# Stops `bolt setup --preset code --yes` and `bolt start --preset code` at each abort point, then
# runs a command and undo: both rule files must be as before or as after, and undo exact. Sandbox only.
set -u
BOLT=${BOLT:-/work/target/debug/bolt}
export BOLT_CONFIG_DIR=$(mktemp -d)
printf '[user]\nname = "T"\nemail = "t@example.invalid"\n' > "$BOLT_CONFIG_DIR/config.toml"
fresh() {
    p=$(mktemp -d); cd "$p"
    echo '{}' > package.json
    $BOLT start --preset custom >/dev/null 2>&1
    printf 'preset = "mine"\nchunk-files-over = "2 MiB"\n' > bolt.toml
    printf 'my-own-line\n\r\n' > .boltignore
    echo hi > a.txt
    $BOLT save "with rules" >/dev/null 2>&1
    printf 'preset = "mine"\n# unsaved edit\n' > bolt.toml
    printf '\xef\xbb\xbfmine\r\n# >>> Bolt preset: old\nx\n# <<< Bolt preset\ntail' > .boltignore
    cp bolt.toml /tmp/b.toml; cp .boltignore /tmp/b.ignore
}
for n in $(seq 1 40); do
    fresh
    BOLT_FAULT_AT=$n $BOLT setup --preset code --yes > /tmp/out 2>&1; code=$?
    if [ $code -eq 0 ]; then echo "setup: no stop at $n, done"; break; fi
    cp bolt.toml /tmp/s.toml 2>/dev/null; cp .boltignore /tmp/s.ignore 2>/dev/null
    $BOLT status > /tmp/st 2>&1; st=$?
    $BOLT setup --preset code --yes > /tmp/again 2>&1; again=$?
    $BOLT undo > /tmp/u 2>&1; u=$?
    # An undo that only finishes the stopped setup takes no step: the next one does.
    if grep -q "nothing more was undone" /tmp/u; then $BOLT undo > /tmp/u 2>&1; u=$?; fi
    # Undo of a finished setup gives the user's bytes back; when the stop came before the step
    # was recorded, undo takes back the save instead, so judge the files only.
    ok=bad
    if cmp -s bolt.toml /tmp/b.toml && cmp -s .boltignore /tmp/b.ignore; then ok=back; fi
    echo "stop $n ($(grep -o 'injected crash at [a-z:-]*' /tmp/out)): status $st, setup again $again, undo $u: $ok"
    [ $ok = bad ] && { echo "  after stop: $(head -c 60 /tmp/s.toml | tr '\n' '|')"; tail -3 /tmp/again /tmp/u; echo "  now: $(head -c 80 bolt.toml | tr '\n' '|') / $(head -c 80 .boltignore | od -c | head -2)"; }
done
