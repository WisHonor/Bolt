#!/usr/bin/env bash
# RT14: a blob over 1,024 bytes whose first 1,024 hold a Git LFS pointer. Git LFS's smudge filter
# reads only those and checks out the object; Bolt reads the blob as an ordinary file. Sandbox only.
set -eu
export PATH=/work/target/release:$PATH
W=$(mktemp -d); cd "$W"
git init -q repo; cd repo
git lfs install --local >/dev/null
printf 'hello world\n' > obj
OID=$(sha256sum obj | cut -d' ' -f1)
mkdir -p .git/lfs/objects/${OID:0:2}/${OID:2:2}
cp obj .git/lfs/objects/${OID:0:2}/${OID:2:2}/$OID; rm obj
printf '* filter=lfs diff=lfs merge=lfs -text\n' > .gitattributes
# The pointer, padded with spaces to 1,024 bytes, then a tail of anything.
{ printf 'version https://git-lfs.github.com/spec/v1\noid sha256:%s\nsize 12\n' "$OID"; } > p
pad=$((1024 - $(wc -c < p))); head -c $pad /dev/zero | tr '\0' ' ' >> p
printf 'TAIL: anything at all\n' >> p
echo "blob bytes: $(wc -c < p)"
B=$(git hash-object -w p); rm p
git update-index --add --cacheinfo 100644,$B,file.bin
git add .gitattributes
git commit -qm one
rm -f file.bin; git checkout -- file.bin
echo "git lfs checkout gives: $(head -c 40 file.bin | od -c | head -2 | tr -s ' ')"
git lfs ls-files
cd ..; rm -rf repo/file.bin
cd repo && bolt import >/dev/null 2>&1 || bolt import 2>&1 | tail -3
echo "bolt status after import:"; bolt status 2>&1 | head -5
echo "bolt saved file.bin as: $(bolt show --json 2>/dev/null | head -c 0)"; bolt goto main >/dev/null 2>&1 || true
ls -la file.bin; head -c 60 file.bin | od -c | head -3
