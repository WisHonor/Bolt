#!/usr/bin/env bash
# Tries a release program on a FAT32 and an exFAT drive (a loop file each, mounted for this
# user), in a folder and at the top. Prints what Opalro says; a refusal is written down, not a
# failure. Fails only if a try can't be set up.
# Usage: ordinary-computers.sh <opalro>
set -euo pipefail

opalro=$1
work=$(mktemp -d)
export OPALRO_CONFIG_DIR="$work/config"
config="$OPALRO_CONFIG_DIR/config.toml"
user='[user]
name = "Ana Lima"
email = "ana@example.com"'
unset OPALRO_USER_NAME OPALRO_USER_EMAIL
mkdir "$OPALRO_CONFIG_DIR"

# Runs opalro and prints all it said and its exit code; what it said is also in $work/said.
try_opal() {
  echo "> opalro $*"
  local s=0
  "$opalro" "$@" > "$work/said" 2>&1 || s=$?
  sed 's/^/  /' "$work/said"
  echo "  (exit $s)"
}

# Start, two saves, a goto back to the first, verify. What start and the first save said
# goes to $work/case.
use_project() {
  cd "$1"
  try_opal start --preset custom
  cat "$work/said" > "$work/case"
  echo one > a.txt
  try_opal save First
  cat "$work/said" >> "$work/case"
  echo two > a.txt
  try_opal save Second
  try_opal history --json
  first=$(jq -r '.saves[1].change_id // empty' "$work/said" 2> /dev/null || true)
  if [ -n "$first" ]; then
    try_opal goto "$first"
    echo "  a.txt now holds: $(cat a.txt)"
  fi
  try_opal verify
}

# The case, then, if Opalro offered a safe-folders line, the case again with that line added.
try_case() {
  echo
  echo "::group::$1 ($2)"
  echo "$user" > "$config"
  mkdir -p "$2"
  use_project "$2"
  line=$(grep -m 1 -E '^\s*safe-folders = ' "$work/case" | sed 's/^ *//' || true)
  if [ -n "$line" ]; then
    echo "-- again, with the hint's line at the top of the settings: $line"
    printf '%s\n%s\n' "$line" "$user" > "$config"
    use_project "$2"
  else
    echo "-- Opalro offered no safe-folders line"
  fi
  cd "$work"
  echo "::endgroup::"
}

echo "Running as: $(id)"
skipped=()
for fs in vfat exfat; do
  if ! command -v "mkfs.$fs" > /dev/null; then
    echo
    echo "::warning::$fs: not run, this runner has no mkfs.$fs"
    skipped+=("$fs")
    continue
  fi
  img="$work/$fs.img" mnt="$work/$fs"
  truncate -s 512M "$img"
  if [ "$fs" = vfat ]; then mkfs.vfat -F 32 "$img"; else mkfs.exfat "$img"; fi
  mkdir "$mnt"
  sudo mount -o "loop,uid=$(id -u),gid=$(id -g)" -t "$fs" "$img" "$mnt"
  findmnt "$mnt"
  try_case "$fs, a folder" "$mnt/proj"
  try_case "$fs, the top of the drive" "$mnt"
  sudo umount "$mnt"
done
echo
if [ ${#skipped[@]} -eq 0 ]; then
  echo "ordinary-computers: every try ran"
else
  echo "ordinary-computers: not run on ${skipped[*]}"
fi
