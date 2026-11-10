#!/usr/bin/env bash
# Times Bolt, Git, and Git with Git LFS through the same steps on each data set (bench/README.md).
#   run.sh sets <folder for the data> <folder for the results>
#   run.sh summary <folder for the results>
#   run.sh import <Git repository> <bolt> <another bolt>
# Needs bolt, git, git-lfs, hyperfine, jq, curl and openssl on the PATH.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
export BOLT_USER_NAME=Bench BOLT_USER_EMAIL=bench@example.com
export GIT_AUTHOR_NAME=Bench GIT_AUTHOR_EMAIL=bench@example.com
export GIT_COMMITTER_NAME=Bench GIT_COMMITTER_EMAIL=bench@example.com
BOLT_CONFIG_DIR=$(mktemp -d)
export BOLT_CONFIG_DIR
# Only this folder goes on exit; the data folder is the caller's, and stays after a failure too.
trap 'rm -rf "$BOLT_CONFIG_DIR"' EXIT

# hyperfine runs each command with bash -c; on Windows, Git's own bash, named by its path.
if command -v cygpath >/dev/null; then shell="'$(cygpath -m "$BASH")'"; else shell=bash; fi
count=0

# step <name> <prepare> <command>: one warm-up run, then 5 timed runs, or 3 if the warm-up took
# over a minute. Before each, untimed: Git's background housekeeping is waited for, the prepare
# command runs, and everything written is flushed to the disk, so no run pays for an earlier one.
step() {
	local name=$1 prepare began runs=5 out command=$3
	prepare="bash '$here/data.sh' settle .${2:+ && $2} && sync"
	eval "$prepare" >/dev/null
	began=$(date +%s)
	eval "$command" >/dev/null
	if [ $(($(date +%s) - began)) -gt 60 ]; then runs=3; fi
	count=$((count + 1))
	out=$results/$(printf %03d "$count")-$name
	hyperfine --style basic --shell "$shell" --runs "$runs" --prepare "$prepare" \
		--command-name "$name" --export-json "$out.json" --export-markdown "$out.md" "$command"
}

# size <name> <folder>: the bytes of the files in a folder, once Git's housekeeping is done.
size() {
	bash "$here/data.sh" settle .
	printf '%s\t%s\n' "$1" "$(du -sb "$2" | cut -f1)" | tee -a "$results/sizes.tsv"
}

# steps <set> <bolt, git or lfs>: every step for one tool, on a copy of the set.
steps() {
	local set=$1 t=$2 dir=$work/$1-$2 n=$work/$1-$2.n start save status big first
	rm -rf "$dir" "$n" && cp -a "$work/$set" "$dir" && cd "$dir"
	if [ "$t" = bolt ]; then
		start='rm -rf .bolt && bolt start --preset custom'
		save="bolt save 'A save' --yes"
		status='bolt status'
	else
		start='rm -rf .git && git init -q -b main && git config core.autocrlf false'
		save="git add -A && git commit -q -m 'A commit'"
		status='git status'
	fi
	if [ "$t" = lfs ]; then
		# Git LFS tracks every file of at least 1 MiB, by name.
		start="$start && git lfs install --local"
		git init -q
		find . -type f -size +1048575c -not -path './.git/*' | sed 's|^\./||' |
			xargs -r -d '\n' git lfs track --filename >/dev/null
	fi
	step "$set-$t-first-save" "$start" "$save"
	step "$set-$t-small-change" "bash '$here/data.sh' change $set . '$n'" "$save"
	if [ "$set" = versions ]; then
		# The rest of the 10 versions.
		while [ "$(cat "$n")" -lt 9 ]; do
			bash "$here/data.sh" change versions . "$n"
			eval "$save" >/dev/null
		done
	fi
	# Bolt's cache doesn't vouch for a file changed in the last 2 seconds, nor Git's index for one
	# changed in the second it was written, so the files age first.
	sleep 3
	step "$set-$t-status" "" "$status"
	big=$(find . -name .git -prune -o -name .bolt -prune -o -type f -printf '%s %p\n' | sort -n | tail -1 | cut -d' ' -f2-)
	step "$set-$t-status-after-touching" "touch '$big'" "$status"
	if [ "$t" = bolt ]; then
		# The save before it is the empty one bolt start makes.
		first=$(bolt history --json | jq -j '.saves[-2].id')
		step "$set-$t-back-to-first" 'bolt goto main' "bolt goto $first"
		size "$set-$t" .bolt
	else
		first=$(git rev-list --max-parents=0 HEAD)
		step "$set-$t-back-to-first" 'git checkout -q main' "git checkout -q $first"
		size "$set-$t" .git
	fi
	cd "$work" && rm -rf "$dir" "$n"
}

# imported <name>: times bolt import of the Git repository in the current folder, then sizes both.
imported() {
	step "$1-import" 'rm -rf .bolt' 'bolt import'
	size "$1-git" .git
	size "$1-bolt" .bolt
}

case $1 in
sets)
	work=$2 results=$3
	mkdir -p "$work" "$results"
	for set in code godot noise versions repeated; do
		[ -d "$work/$set" ] || bash "$here/data.sh" "$set" "$work/$set"
		# An hour old, so no tool takes a file for one that may still be changing.
		find "$work/$set" -type f -exec touch -d '1 hour ago' {} +
		size "$set-files" "$work/$set"
		for t in bolt git lfs; do steps "$set" "$t"; done
		rm -rf "${work:?}/$set"
	done
	bash "$here/data.sh" lfs "$work/lfs"
	cd "$work/lfs" && imported lfs && size lfs-git-lfs-objects .git/lfs
	cd "$work" && rm -rf "$work/lfs"
	bash "$here/data.sh" history "$work/history" "$work/history.bin"
	cd "$work/history" && imported history
	cd "$work" && rm -rf "$work/history"
	;;
summary)
	echo "== Times in seconds: step, mean, standard deviation, min, max, runs"
	for f in "$2"/*.json; do
		jq -r '.results[] | [.command, .mean, .stddev, .min, .max, (.times | length)] | @tsv' "$f"
	done
	echo "== Sizes in bytes"
	cat "$2/sizes.tsv"
	;;
import)
	# Imports the repository with each bolt in turn, three times, and after each import times the
	# first save, of a small change. Git wrote some files with CRLF, as .gitattributes asked, so
	# the save takes them as they are.
	cd "$2"
	echo "== $(git log -1 --format='%H %cs') ($(git rev-list --all --count) commits, $(git for-each-ref | wc -l) refs)"
	for i in 1 2 3; do
		for bolt in "$3" "$4"; do
			rm -rf .bolt
			git checkout -q -- README.md
			/usr/bin/time -f "$bolt, import $i: %e s, peak %M KiB" "$bolt" import >/dev/null
			echo "A small change." >>README.md
			/usr/bin/time -f "$bolt, first save $i: %e s, peak %M KiB" "$bolt" save "A change" --yes --accept-crlf >/dev/null
		done
	done
	;;
*)
	echo "usage: run.sh sets <data folder> <results folder>, run.sh summary <results folder>," >&2
	echo "       or run.sh import <repository> <bolt> <another bolt>" >&2
	exit 2
	;;
esac
