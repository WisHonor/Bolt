#!/usr/bin/env bash
# Milestone 5 red team: what Git LFS itself (3.3.0 in the sandbox) takes for a pointer, and where
# it takes lfs.storage from. Run only inside the sandbox:
#   bash redteam/sandbox.sh run bash redteam/attacks7/git-lfs-probes.sh
set -euo pipefail
dir=$(mktemp -d)
cd "$dir"
export HOME="$dir" GIT_CONFIG_NOSYSTEM=1
git init -q .
git lfs install --local >/dev/null
oid=$(printf hello | sha256sum | cut -c1-64)
pointer() { printf 'version https://git-lfs.github.com/spec/v1\noid sha256:%s\nsize 5\n' "$1"; }

echo "== oid spellings (git lfs pointer --check)"
for o in "$oid" "$(echo "$oid" | tr a-f A-F)" "${oid}xyz"; do
    pointer "$o" > p
    printf '%-70s ' "$o"
    git lfs pointer --check --file=p >/dev/null 2>&1 && echo pointer || echo not-a-pointer
done
printf '  \r\n\r\nversion https://git-lfs.github.com/spec/v1\r\n\r\noid sha256:%s\r\nsize 5\r\n  \n' "$oid" > p
git lfs pointer --check --file=p >/dev/null 2>&1 && echo "CRLF, blank lines and spaces: pointer"

echo "== a pointer padded past 1,024 bytes, with anything after (RT8-3)"
{ pointer "$oid"; head -c 3000 /dev/zero | tr '\0' ' '; printf 'TAIL AFTER 1024\n'; } > padded
mkdir -p ".git/lfs/objects/${oid:0:2}/${oid:2:2}"
printf hello > ".git/lfs/objects/${oid:0:2}/${oid:2:2}/$oid"
git lfs pointer --check --file=padded >/dev/null 2>&1 && echo "$(wc -c < padded) bytes: pointer"
echo "smudged: $(git lfs smudge < padded)"

echo "== where lfs.storage comes from (RT8-1)"
printf '[lfs]\n\tstorage = /tmp/elsewhere-lfs\n' > .lfsconfig
echo ".lfsconfig:          $(git lfs env 2>/dev/null | grep LocalMediaDir)"
rm .lfsconfig
echo "GIT_CONFIG_COUNT:    $(GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=lfs.storage GIT_CONFIG_VALUE_0=/tmp/env-lfs git lfs env 2>/dev/null | grep LocalMediaDir)"
printf '[lfs]\n\tstorage = /tmp/sys-lfs\n' > sysconfig
echo "GIT_CONFIG_SYSTEM:   $(GIT_CONFIG_NOSYSTEM= GIT_CONFIG_SYSTEM=$dir/sysconfig git lfs env 2>/dev/null | grep LocalMediaDir)"
rm -rf "$dir"

echo "== Git LFS's scanner (ls-files, fetch) skips the padded blob that smudge reads (RT8-3)"
dir=$(mktemp -d)
cd "$dir"
export HOME="$dir"
git init -q .
git config user.email a@b.c
git config user.name a
{ pointer "$oid"; head -c 1000 /dev/zero | tr '\0' ' '; printf '\ntail\n'; } > padded.bin
pointer "$oid" > plain.bin
git add -A
git commit -q -m x
echo "ls-files --all: $(git lfs ls-files --all 2>&1 | tr '\n' ' ')"
rm -rf "$dir"
