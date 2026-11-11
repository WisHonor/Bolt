#!/usr/bin/env bash
# Builds honest projects with Bolt's own commands and runs `bolt verify` after every step: any
# problem it reports is a false alarm. Sandbox only.
set -u
BOLT=${BOLT:-/work/target/debug/bolt}
export BOLT_CONFIG_DIR=$(mktemp -d)
printf 'undo-window-days = 1\n[user]\nname = "T"\nemail = "t@example.invalid"\n' > "$BOLT_CONFIG_DIR/config.toml"
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@x GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@x
bad=0
check() {
    out=$($BOLT verify 2>&1); code=$?
    if [ $code -ne 0 ]; then echo "FALSE ALARM after: $1"; echo "$out" | head -20; bad=1; fi
}
r() { $BOLT "$@" >/tmp/r.out 2>&1 || { echo "  ($* exited $?: $(tail -1 /tmp/r.out))"; }; }
noise() { head -c "$2" /dev/urandom > "$1"; }

# 1. A Git repository with links, executables, an empty file and a big file, imported.
p=$(mktemp -d); cd "$p"
git init -q -b main .
echo hi > a.txt; : > empty; printf '#!/bin/sh\n' > run.sh; chmod +x run.sh
ln -s a.txt link; ln -s ../outside up; ln -s "$(printf 'x%.0s' $(seq 1 200))" longlink
mkdir -p d/e; ln -s d dirlink; echo deep > d/e/f
noise big.bin 3000000
git add -A; git commit -qm one
chmod -x run.sh; rm link; ln -s empty link; git add -A; git commit -qm two
r import  || echo "import failed"
check "import"
r goto main ; check "goto after import"

# 2. Saves around the cut threshold, raised and lowered between saves.
noise mid.bin 1500000
r save "mid" --yes ; check "save mid whole"
printf 'chunk-files-over = "1 MiB"\n' > bolt.toml
touch mid.bin; cat mid.bin > mid2.bin
r save "mid chunked" --yes ; check "save mid2 chunked"
printf 'chunk-files-over = "64 MiB"\n' > bolt.toml
cp big.bin big-again.bin; : > empty2
r save "big whole" --yes ; check "save big whole"
printf 'chunk-files-over = "1 MiB"\n' > bolt.toml
noise grow.bin 1048576; r save "exactly 1 MiB" --yes ; check "1 MiB file"
head -c 1 /dev/zero >> grow.bin; r save "1 MiB + 1" --yes ; check "1 MiB+1"

# 3. Shelves, undo, redo, discard, lines, tidy.
echo change >> a.txt; noise shelved.bin 2500000
r shelf "wip" ; check "shelf"
r unshelf ; check "unshelf"
r shelf "again" 
r undo ; check "undo shelf"
r redo ; check "redo shelf"
r save --line side "on side" 
echo x > side.txt; r save "side" ; check "side line"
r goto main ; check "goto main"
echo y >> a.txt; r discard ; check "discard"
r undo ; check "undo discard"
r save --amend "amended" ; check "amend"
r tidy ; check "tidy"
r setup --preset code --yes ; check "setup code"
r undo ; check "undo setup"
r tidy ; check "tidy 2"
# Age every pack and diary step by moving the clock: not possible here, so age the packs.
find .bolt/store/packs -name '*.pack' -exec touch -d '3 days ago' {} +
r tidy ; check "tidy aged"
r goto main ; check "goto after tidy"
r history  || echo "history fails"

# 4. start --preset on a fresh folder with links and executables, then everything again.
q=$(mktemp -d); cd "$q"
echo '{}' > package.json; ln -s package.json pj; printf 'x' > one; chmod +x one
r start --preset code ; check "start code"
r setup --preset game --yes ; check "setup game"
noise art.psd 5000000; r save "art" --yes ; check "save game art"
r undo ; check "undo save art"
r tidy ; check "tidy undone art"
echo "done; false alarms: $bad"
