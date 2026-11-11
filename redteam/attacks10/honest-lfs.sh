#!/usr/bin/env bash
# Imports honest Git repositories with Git LFS files (object present, and pointer only), a
# submodule entry and files Git converts on checkout, then runs `bolt verify` and `bolt goto`:
# any problem is a false alarm. Sandbox only.
set -u
BOLT=${BOLT:-/work/target/debug/bolt}
export BOLT_CONFIG_DIR=$(mktemp -d) HOME=$(mktemp -d)
printf '[user]\nname = "T"\nemail = "t@example.invalid"\n' > "$BOLT_CONFIG_DIR/config.toml"
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@x GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@x
git config --global init.defaultBranch main
git config --global protocol.file.allow always
git lfs install >/dev/null 2>&1 || echo "no git lfs"
check() {
    out=$($BOLT verify 2>&1); code=$?
    if [ $code -ne 0 ]; then echo "FALSE ALARM after: $1"; echo "$out" | head -20; fi
    echo "$1: verify $code"
}
sub=$(mktemp -d); (cd "$sub" && git init -q && echo s > s && git add s && git commit -qm s)
p=$(mktemp -d); cd "$p"
git init -q
git lfs track '*.bin' >/dev/null
head -c 3000000 /dev/urandom > big.bin
head -c 100 /dev/urandom > small.bin
printf 'a\r\nb\r\n' > crlf.txt
printf '* text=auto\n*.bin filter=lfs diff=lfs merge=lfs -text\n' > .gitattributes
git add -A; git commit -qm lfs
git -c protocol.file.allow=always submodule add -q "$sub" mod 2>/dev/null; git commit -qm sub
$BOLT import > /tmp/imp 2>&1 || { echo "import failed:"; tail -3 /tmp/imp; }
check "import with LFS objects"
$BOLT goto main >/dev/null 2>&1; echo "goto: $?"

# The LFS object of one file missing locally: only its pointer.
q=$(mktemp -d); cd "$q"
git init -q; git lfs track '*.bin' >/dev/null
head -c 2000000 /dev/urandom > gone.bin
git add -A; git commit -qm lfs
rm -rf .git/lfs/objects
$BOLT import > /tmp/imp 2>&1; echo "import (object missing): $? $(tail -1 /tmp/imp)"
check "import with an LFS object missing"
