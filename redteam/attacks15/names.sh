#!/usr/bin/env bash
# RT16 (decision 024): the widened short-name rule through the release program. Sandbox only.
# Uses $OPALRO, else /work/target/release/opalro (cargo build --release -p opal-vcs).
set -u
B=${OPALRO:-/work/target/release/opalro}
T=$(mktemp -d)
export OPALRO_CONFIG_DIR=$T/config OPALRO_USER_NAME=Red OPALRO_USER_EMAIL=red@example.invalid
export GIT_AUTHOR_NAME=g GIT_AUTHOR_EMAIL=g@x GIT_COMMITTER_NAME=g GIT_COMMITTER_EMAIL=g@x
mkdir -p "$T/config" "$T/save/sub"

echo "== save: each name at the top and in sub/"
cd "$T/save" && "$B" start --preset custom >/dev/null 2>&1
names=("opalro~5" "OPALRO~9." "op9975~1" "OP997~12 " "o~123456" "~1234567" "op9975~1:x"
    $'op\xe2\x80\x8c9975~1' $'~\xe2\x80\x8f1234567' "~\$doc.docx" "~1234567.txt" "op9975~10"
    "opalr~1" "opalr~10" "~WRL0001.tmp")
for n in "${names[@]}"; do printf x > "$n"; printf x > "sub/$n"; done
"$B" save "Names" 2>&1
echo "-- what the save holds, written back:"
rm -rf sub && for n in "${names[@]}"; do rm -f "$n"; done
"$B" discard >/dev/null 2>&1
find . -path ./.opalro -prune -o -type f -print | LC_ALL=C sort

echo "== import: Git tracks each name at the top"
for n in "opalro~5" "op9975~1" "~1234567" "o~123456" "opalr~1"; do
    d=$(mktemp -d) && cd "$d" && git init -q && printf x > "$n" && git add -A && git commit -qm one
    printf '   %-10s %s\n' "$n" "$("$B" import 2>&1 | head -1)"
done
echo "== import: Git tracks each name in sub/"
for n in "opalro~6" "op9975~2" "~1234567"; do
    d=$(mktemp -d) && cd "$d" && git init -q && mkdir sub && printf x > "sub/$n" && printf y > keep
    git add -A && git commit -qm one
    printf '   %-14s %s\n' "sub/$n" "$("$B" import 2>&1 | grep -i 'left out' | head -1)"
done
rm -rf "$T"
