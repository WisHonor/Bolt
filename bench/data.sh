#!/usr/bin/env bash
# Builds the benchmark data sets (bench/README.md). Every byte comes from a fixed tag or a fixed
# seed, so each run builds the same data.
set -euo pipefail

CODE_TAG=v2.56.0
GODOT_TAG=4.7-6ad6167
# BENCH_MIB=1024 makes every generated set 1,024 times smaller, to try the scripts quickly.
MIB=${BENCH_MIB:-1048576}

# $1 bytes of AES-256-CTR keystream keyed by the text $2: the same bytes for the same text, and
# nothing in them compresses.
noise() {
	local key
	key=$(printf '%s' "$2" | sha256sum | cut -c1-64)
	head -c "$1" /dev/zero | openssl enc -aes-256-ctr -nosalt -K "$key" -iv 00000000000000000000000000000000
}

# Overwrites $3 bytes of the file $1 at byte $2 with noise keyed by $4.
overwrite() {
	noise "$3" "$4" | dd of="$1" bs=64K seek="$2" oflag=seek_bytes conv=notrunc iflag=fullblock status=none
}

# Downloads a tag's files from GitHub into $2, without its history.
fetch() {
	mkdir -p "$2"
	curl -fsSL --retry 5 "https://github.com/$1.tar.gz" | tar -xz -C "$2" --strip-components=1
}

case $1 in
code) fetch "git/git/archive/refs/tags/$CODE_TAG" "$2" ;;
godot) fetch "godotengine/godot-demo-projects/archive/refs/tags/$GODOT_TAG" "$2" ;;
noise)
	mkdir -p "$2"
	for i in $(seq -w 1 20); do noise $((50 * MIB)) "noise $i" >"$2/noise-$i.bin"; done
	;;
versions)
	mkdir -p "$2"
	noise $((1024 * MIB)) versions >"$2/big.bin"
	;;
repeated)
	# One mebibyte of noise, 1,024 times over.
	mkdir -p "$2"
	noise "$MIB" repeated >"$2/big.bin"
	for _ in $(seq 10); do cat "$2/big.bin" "$2/big.bin" >"$2/twice" && mv "$2/twice" "$2/big.bin"; done
	;;
settle)
	# Waits for the housekeeping Git 2.54 and later run in the background after a commit.
	while [ -e "$2/.git/objects/maintenance.lock" ] || [ -e "$2/.git/gc.pid" ]; do sleep 1; done
	;;
change)
	# change <set> <folder> <counter file>: the set's next small change, numbered from 1.
	n=$(($(cat "$4" 2>/dev/null || echo 0) + 1))
	echo "$n" >"$4"
	case $2 in
	code | godot) printf 'A small change, number %s.\n' "$n" >>"$3/README.md" ;;
	noise) overwrite "$3/noise-01.bin" $((25 * MIB + MIB / 3)) 4096 "noise change $n" ;;
	repeated) overwrite "$3/big.bin" $((512 * MIB + MIB / 3)) 4096 "repeated change $n" ;;
	versions)
		# Version n + 1: a mebibyte overwritten and 4 KiB inserted, at offsets fixed for each version.
		f=$3/big.bin
		overwrite "$f" $((n * 97 * MIB + MIB / 3)) "$MIB" "versions overwrite $n"
		at=$((n * 89 * MIB + MIB / 5))
		{ head -c "$at" "$f"; noise 4096 "versions insert $n"; tail -c +$((at + 1)) "$f"; } >"$f.new"
		mv "$f.new" "$f"
		;;
	*) echo "no small change for $2" >&2 && exit 2 ;;
	esac
	;;
lfs)
	# A Git LFS repository: five files of 100 MiB in 10 versions, each version overwriting a
	# mebibyte of each file.
	mkdir -p "$2" && cd "$2"
	git init -q -b main
	git config core.autocrlf false
	git lfs install --local >/dev/null
	git lfs track '*.bin' >/dev/null
	for f in 1 2 3 4 5; do noise $((100 * MIB)) "lfs $f" >"asset-$f.bin"; done
	git add -A && git commit -q -m "Version 1"
	for v in $(seq 2 10); do
		for f in 1 2 3 4 5; do
			overwrite "asset-$f.bin" $(((v * 7 + f * 3) % 99 * MIB + MIB / 3)) "$MIB" "lfs $v $f"
		done
		git add -A && git commit -q -m "Version $v"
	done
	;;
history)
	# A Git repository of one 200 MiB file in 50 versions, each overwriting 1% of it (eight runs of
	# 256 KiB), written by git fast-import, which stores each version as a delta on the one before.
	# The file being edited is $3, outside the repository; it is deleted afterwards.
	git init -q -b main "$2"
	noise $((200 * MIB)) history >"$3"
	for v in $(seq 1 50); do
		if [ "$v" -gt 1 ]; then
			for r in 1 2 3 4 5 6 7 8; do
				overwrite "$3" $(((v * 8 + r) * 25000009 % (200 * MIB - MIB / 4))) $((MIB / 4)) "history $v $r"
			done
		fi
		message="Version $v"
		printf 'commit refs/heads/main\ncommitter Bench <bench@example.com> %s +0000\n' $((1700000000 + v))
		printf 'data %s\n%s\nM 644 inline big.bin\ndata %s\n' ${#message} "$message" $((200 * MIB))
		cat "$3"
		printf '\n'
	done | git -C "$2" fast-import --quiet
	rm "$3"
	;;
*)
	echo "usage: data.sh code|godot|noise|versions|repeated|lfs|settle <folder>" >&2
	echo "       data.sh history <folder> <file>, data.sh change <set> <folder> <counter file>" >&2
	exit 2
	;;
esac
