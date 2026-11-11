#!/usr/bin/env bash
# Takes a release program through everyday commands in a fresh place, with the project and
# the settings in folders whose names hold spaces and letters outside ASCII.
# Usage: clean-machine.sh <bolt> <version> <git-repo>
# Stops at the first step that fails, and names it.
set -eEuo pipefail

bolt=$1 version=$2 gitrepo=$3
current=""
trap 'echo "clean-machine FAILED at: $current" >&2' ERR
step() { current=$1; echo; echo "== $1"; }

work=$(mktemp -d)
if command -v cygpath > /dev/null; then work=$(cygpath -m "$work"); fi
root="$work/Projets d'été/日本"
export BOLT_CONFIG_DIR="$work/réglages de bolt"
unset BOLT_USER_NAME BOLT_USER_EMAIL
mkdir -p "$root" "$BOLT_CONFIG_DIR"
printf '[user]\nname = "Clean Machine"\nemail = "clean@example.invalid"\n' > "$BOLT_CONFIG_DIR/config.toml"

step "bolt --version prints bolt $version"
said=$("$bolt" --version)
echo "$said"
[ "$said" = "bolt $version" ]

for preset in code game custom; do
  step "start --preset $preset in a new folder"
  mkdir "$root/$preset"
  cd "$root/$preset"
  "$bolt" start --preset "$preset"
done

cd "$root/code"
step "a file, then save First"
echo "first" > notes.txt
"$bolt" save "First"

step "a change, and status names it"
echo "second" > notes.txt
"$bolt" status | tee "$work/status.txt"
grep -q notes.txt "$work/status.txt"

step "save Second"
"$bolt" save "Second"

step "goto the first save, by the change ID history --json gives"
# Newest first: Second, then First.
first=$("$bolt" history --json | grep -o '"change_id": *"[k-z]*"' | sed -n '2s/.*"\([k-z]*\)"$/\1/p')
echo "First is $first"
[ -n "$first" ]
"$bolt" goto "$first"

step "the file holds the first contents"
[ "$(cat notes.txt)" = first ]

step "undo: the second contents are back"
"$bolt" undo
[ "$(cat notes.txt)" = second ]

step "redo: the first contents again"
"$bolt" redo
[ "$(cat notes.txt)" = first ]

step "a change, put aside with shelf: the folder is clean again"
echo "third" > notes.txt
"$bolt" shelf "Third"
[ "$(cat notes.txt)" = first ]
"$bolt" status --json | tee "$work/status.json"
grep -Eq '"changes": *\[\]' "$work/status.json"

step "verify"
"$bolt" verify

step "import a Git repository into a copy of it"
cp -R "$gitrepo" "$root/imported"
cd "$root/imported"
"$bolt" import

step "history of the import"
"$bolt" history | tee "$work/history.txt"
grep -q "Add notes" "$work/history.txt"

step "verify the import"
"$bolt" verify

cd /
rm -rf "$work"
echo
echo "clean-machine passed: $("$bolt" --version)"
