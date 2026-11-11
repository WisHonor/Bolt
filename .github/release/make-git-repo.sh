#!/usr/bin/env bash
# Makes a small Git repository for Bolt to import: three commits on two branches, a tag,
# a subfolder and a 2 MiB file. None of this computer's Git settings are read.
# Usage: make-git-repo.sh <dir>
set -euo pipefail

dir=$1
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME="Clean Machine" GIT_AUTHOR_EMAIL="clean@example.invalid"
export GIT_COMMITTER_NAME="Clean Machine" GIT_COMMITTER_EMAIL="clean@example.invalid"

commit() {
  export GIT_AUTHOR_DATE="$1" GIT_COMMITTER_DATE="$1"
  git add -A
  git commit -q -m "$2"
}

mkdir -p "$dir"
cd "$dir"
git init -q -b main
git config core.autocrlf false

echo "A small project" > README.md
mkdir -p src assets
echo "fn main() {}" > src/main.rs
head -c 2097152 /dev/urandom > assets/big.bin
commit "2026-01-01T10:00:00Z" "First commit"
git tag v1

git checkout -q -b feature
echo "fn main() { println!(\"hi\"); }" > src/main.rs
commit "2026-01-02T10:00:00Z" "Say hi"

git checkout -q main
echo "Notes" > notes.txt
commit "2026-01-03T10:00:00Z" "Add notes"

git log --all --oneline --decorate
