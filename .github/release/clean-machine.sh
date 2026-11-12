#!/usr/bin/env bash
# Takes a release program through everyday commands in a fresh place, with the project and
# the settings in folders whose names hold spaces and letters outside ASCII.
# Usage: clean-machine.sh <opal> <version> <git-repo>
# Stops at the first step that fails, and names it.
set -eEuo pipefail

opal=$1 version=$2 gitrepo=$3
current=""
trap 'echo "clean-machine FAILED at: $current" >&2' ERR
step() { current=$1; echo; echo "== $1"; }

work=$(mktemp -d)
if command -v cygpath > /dev/null; then work=$(cygpath -m "$work"); fi
root="$work/Projets d'été/日本"
export OPAL_CONFIG_DIR="$work/réglages de opal"
unset OPAL_USER_NAME OPAL_USER_EMAIL
mkdir -p "$root" "$OPAL_CONFIG_DIR"
printf '[user]\nname = "Clean Machine"\nemail = "clean@example.invalid"\n' > "$OPAL_CONFIG_DIR/config.toml"

step "opal --version prints opal $version"
said=$("$opal" --version)
echo "$said"
[ "$said" = "opal $version" ]

for preset in code game custom; do
  step "start --preset $preset in a new folder"
  mkdir "$root/$preset"
  cd "$root/$preset"
  "$opal" start --preset "$preset"
done

cd "$root/code"
step "a file, then save First"
echo "first" > notes.txt
"$opal" save "First"

step "a change, and status names it"
echo "second" > notes.txt
"$opal" status | tee "$work/status.txt"
grep -q notes.txt "$work/status.txt"

step "save Second"
"$opal" save "Second"

step "goto the first save, by the change ID history --json gives"
# Newest first: Second, then First.
first=$("$opal" history --json | grep -o '"change_id": *"[k-z]*"' | sed -n '2s/.*"\([k-z]*\)"$/\1/p')
echo "First is $first"
[ -n "$first" ]
"$opal" goto "$first"

step "the file holds the first contents"
[ "$(cat notes.txt)" = first ]

step "undo: the second contents are back"
"$opal" undo
[ "$(cat notes.txt)" = second ]

step "redo: the first contents again"
"$opal" redo
[ "$(cat notes.txt)" = first ]

step "a change, put aside with shelf: the folder is clean again"
echo "third" > notes.txt
"$opal" shelf "Third"
[ "$(cat notes.txt)" = first ]
"$opal" status --json | tee "$work/status.json"
grep -Eq '"changes": *\[\]' "$work/status.json"

step "verify"
"$opal" verify

step "import a Git repository into a copy of it"
cp -R "$gitrepo" "$root/imported"
cd "$root/imported"
"$opal" import

step "history of the import"
"$opal" history | tee "$work/history.txt"
grep -q "Add notes" "$work/history.txt"

step "verify the import"
"$opal" verify

cd /
rm -rf "$work"
echo
echo "clean-machine passed: $("$opal" --version)"
