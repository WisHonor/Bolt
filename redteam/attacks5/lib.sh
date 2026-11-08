# Shared set-up for the milestone 4 attack scripts: a fresh project per scenario.
export BOLT_USER_NAME=Test BOLT_USER_EMAIL=test@example.com
export BOLT=/work/target/debug/bolt
export HOME=/tmp/rt-home XDG_CONFIG_HOME=/tmp/rt-home BOLT_CONFIG_DIR=/tmp/rt-home
mkdir -p /tmp/rt-home
b() { "$BOLT" "$@"; }
fresh() { rm -rf "/tmp/rt/$1"; mkdir -p "/tmp/rt/$1"; cd "/tmp/rt/$1" || exit 1; b start >/dev/null; }
show() { echo "--- $*"; }
tree() { find . -path ./.bolt -prune -o -type f -print -o -type l -print | sort | while read -r f; do
    if [ -L "$f" ]; then echo "$f -> $(readlink "$f")"; else echo "$f: $(head -c 60 "$f" | tr '\n' '|')"; fi; done; }
cur() { b --json status | grep -o '"id":"[0-9a-f]*"' | head -1 | cut -d'"' -f4; }
