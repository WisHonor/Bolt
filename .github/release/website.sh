#!/usr/bin/env bash
# Makes the copies of the docs meant for the website, zipped.
# Usage: website.sh <version> <out-zip>
# Fails when a copy still holds the repository's address or a link into it.
set -euo pipefail

version=$1
out=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
repo=${GITHUB_REPOSITORY:-WisHonor/Bolt}

as_is=(docs/releases/download-page.md docs/releases/welcome.md docs/releases/reporting-a-problem.md
    docs/releases/known-problems.md "docs/releases/v$version.md" docs/guide.md USER-AGREEMENT.md SECURITY.md)
unlinked=(docs/json.md docs/storage-format.md docs/benchmarks.md)

dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
mkdir -p "$dir/docs/releases"
for f in "${as_is[@]}"; do cp "$f" "$dir/$f"; done
# [text](link) -> text, for links into the repository: relative ones, and ones to its address.
for f in "${unlinked[@]}"; do
    REPO=$repo perl -pe 's{\[([^\]]*)\]\((?:(?![a-z][a-z0-9+.-]*:|#)[^)]*|https?://(?:www\.)?github\.com/\Q$ENV{REPO}\E(?:[/#?][^)]*)?)\)}{$1}gi' \
        "$f" > "$dir/$f"
done

bad=0
for f in "${as_is[@]}" "${unlinked[@]}"; do
    # Not -F: with -i it aborts in Git for Windows' grep.
    grep -i -q -e "$repo" "$dir/$f" && s=0 || s=$?
    if [ "$s" = 0 ]; then
        echo "::error::The website copy of $f holds the repository's address"
        bad=1
    elif [ "$s" != 1 ]; then
        echo "::error::grep failed on $f"
        exit 2
    fi
    if ! perl -ne 'exit 1 if /\]\((?![a-z][a-z0-9+.-]*:|#)/i' "$dir/$f"; then
        echo "::error::The website copy of $f holds a link into the repository"
        bad=1
    fi
done
[ "$bad" = 0 ] || exit 1

rm -f "$out"
(cd "$dir" && 7z a -tzip -bd "$out" "${as_is[@]}" "${unlinked[@]}" > /dev/null)
echo "$out holds:"
for f in "${as_is[@]}" "${unlinked[@]}"; do echo "  $f"; done
