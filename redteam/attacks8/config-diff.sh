#!/usr/bin/env bash
# Milestone 5 red team, session 2: which files Git keeps out with the user's own ignore list
# (core.excludesFile) against which files Bolt keeps out, under odd settings of the variables that
# place the user's Git settings. Run only inside the sandbox:
#   bash redteam/sandbox.sh run bash redteam/attacks8/config-diff.sh [scenario...]
set -uo pipefail
BOLT=${BOLT:-/work/target/release/bolt}
base=$(mktemp -d)
export BOLT_CONFIG_DIR=$(mktemp -d) BOLT_USER_NAME=T BOLT_USER_EMAIL=t@example.com
unset GIT_CONFIG_SYSTEM GIT_CONFIG_GLOBAL GIT_CONFIG_COUNT GIT_CONFIG_PARAMETERS GIT_CONFIG
export GIT_CONFIG_NOSYSTEM=1

# A fresh project with a.key, b.psd, c.txt and d.log, and a home folder; prints its folder.
fresh() {
    local d
    d=$(mktemp -d -p "$base")
    mkdir -p "$d/home" "$d/proj"
    (cd "$d/proj" && git init -q . && printf a > a.key && printf b > b.psd && printf c > c.txt \
        && printf d > d.log && HOME=$d/home "$BOLT" start >/dev/null 2>&1)
    echo "$d"
}

# What Git's status lists as new, then what Bolt's does, in `proj`, with the environment the
# caller set; prints both and whether they agree.
compare() {
    local name=$1 proj=$2
    local g b
    # Git blocks opening a list that is a pipe, so each side gets 10 seconds.
    g=$(cd "$proj" && timeout 10 git status --porcelain -uall 2>&1 | grep -av '\.bolt/' | sed 's/^?? //' | sort | tr '\n' ' ')
    b=$(cd "$proj" && timeout 10 "$BOLT" --json status 2>&1 | python3 -c '
import json, sys
t = sys.stdin.read()
try:
    d = json.loads(t)
    print(" ".join(sorted(c["path"] for c in d["changes"] if c["change"] == "added")))
except Exception:
    print("STOP: " + " ".join(t.split())[:300])
')
    if [ "$g " = "$b " ] || [ "$g" = "$b " ] || [ "$g" = "$b" ]; then v=same; else v=DIFFERENT; fi
    printf '%-46s %-9s git: %-26s bolt: %s\n' "$name" "$v" "$g" "$b"
}

run() { "$@"; }

scenario() {
    local d
    case $1 in
    home-unset)
        # Git reads no ~/.gitconfig and no default list without HOME; Bolt asks the system.
        d=$(fresh)
        local pw
        pw=$(getent passwd "$(id -u)" | cut -d: -f6)
        mkdir -p "$pw/.config/git"
        printf '*.key\n' > "$pw/.config/git/ignore"
        (unset HOME XDG_CONFIG_HOME; compare "HOME unset, ~/.config/git/ignore: *.key" "$d/proj")
        printf '[core]\n\texcludesFile = %s\n' "$d/home/list" > "$pw/.gitconfig"
        printf '*.psd\n' > "$d/home/list"
        (unset HOME XDG_CONFIG_HOME; compare "HOME unset, ~/.gitconfig names *.psd" "$d/proj")
        (unset HOME; export XDG_CONFIG_HOME=$d/home; compare "HOME unset, XDG set, ~/.gitconfig: *.psd" "$d/proj")
        (export HOME=; unset XDG_CONFIG_HOME; compare "HOME empty, ~/.gitconfig names *.psd" "$d/proj")
        rm -rf "$pw/.config/git" "$pw/.gitconfig"
        ;;
    global)
        d=$(fresh)
        printf '*.key\n' > "$d/home/list"
        printf '[core]\n\texcludesFile = %s\n' "$d/home/list" > "$d/home/.gitconfig"
        printf '[core]\n\texcludesFile = %s/home/list2\n' "$d" > "$d/home/g2"
        printf '*.psd\n' > "$d/home/list2"
        export HOME=$d/home XDG_CONFIG_HOME=$d/home
        compare "no variable" "$d/proj"
        (export GIT_CONFIG_GLOBAL=; compare "GIT_CONFIG_GLOBAL empty" "$d/proj")
        (export GIT_CONFIG_GLOBAL=/dev/null; compare "GIT_CONFIG_GLOBAL /dev/null" "$d/proj")
        (export GIT_CONFIG_GLOBAL=/dev//null; compare "GIT_CONFIG_GLOBAL /dev//null" "$d/proj")
        (export GIT_CONFIG_GLOBAL=$d/home/g2; compare "GIT_CONFIG_GLOBAL g2 (*.psd)" "$d/proj")
        (cd "$d/proj" && mkdir -p sub && cp "$d/home/g2" sub/g; export GIT_CONFIG_GLOBAL=sub/g; compare "GIT_CONFIG_GLOBAL relative sub/g" "$d/proj")
        (export GIT_CONFIG_GLOBAL=$d/home/g2/; compare "GIT_CONFIG_GLOBAL g2/ (slash)" "$d/proj")
        (export GIT_CONFIG_GLOBAL="$d/home/./g2"; compare "GIT_CONFIG_GLOBAL ./g2" "$d/proj")
        (export GIT_CONFIG_GLOBAL="~/g2"; compare "GIT_CONFIG_GLOBAL ~/g2 (literal)" "$d/proj")
        (export GIT_CONFIG_GLOBAL=$d/home; compare "GIT_CONFIG_GLOBAL a folder" "$d/proj")
        (export GIT_CONFIG_GLOBAL=$d/home/missing; compare "GIT_CONFIG_GLOBAL missing" "$d/proj")
        (export GIT_CONFIG_GLOBAL=$d/home/list/x; compare "GIT_CONFIG_GLOBAL under a file" "$d/proj")
        ;;
    system)
        d=$(fresh)
        printf '*.key\n' > "$d/home/list"
        printf '[core]\n\texcludesFile = %s\n' "$d/home/list" > "$d/home/sys"
        export HOME=$d/home XDG_CONFIG_HOME=$d/home
        unset GIT_CONFIG_NOSYSTEM
        for v in "$d/home/sys" "$d/home//sys" "$d/home/./sys" "$d/home/x/../sys" "$d/home/sys/" \
                 "$d/home/sys/." "$d/home/sys/.." "/dev/null" "/dev/./null" "" "sys-rel"; do
            (export GIT_CONFIG_SYSTEM="$v"; compare "GIT_CONFIG_SYSTEM '$v'" "$d/proj")
        done
        for n in "" 0 1 true yes on no off false 2 -1 " 1" "1 " 0x10 1k 08 TRUE; do
            (export GIT_CONFIG_SYSTEM=$d/home/sys GIT_CONFIG_NOSYSTEM="$n"; compare "NOSYSTEM '$n'" "$d/proj")
        done
        ;;
    count)
        d=$(fresh)
        printf '*.key\n' > "$d/home/list"
        export HOME=$d/home XDG_CONFIG_HOME=$d/home
        for c in "" 1 " 1" "+1" "01" "1 " "-0" "0x1" "2" "4294967297" "-18446744073709551615"; do
            (export GIT_CONFIG_COUNT="$c" GIT_CONFIG_KEY_0=core.excludesFile GIT_CONFIG_VALUE_0=$d/home/list
             compare "COUNT '$c'" "$d/proj")
        done
        for k in core.excludesFile Core.ExcludesFile CORE.EXCLUDESFILE core..excludesFile \
                 'core. .excludesFile' core.excludesfile. .core.excludesFile 'core.excludesFile ' \
                 'core.Excludes-File' 'core.x.excludesFile' 'Core.X.ExcludesFile'; do
            (export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0="$k" GIT_CONFIG_VALUE_0=$d/home/list
             compare "KEY '$k'" "$d/proj")
        done
        (export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.excludesFile GIT_CONFIG_VALUE_0="~/list"
         compare "VALUE ~/list" "$d/proj")
        (cd "$d/proj"; mkdir -p in; printf '*.psd\n' > in/l; export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.excludesFile GIT_CONFIG_VALUE_0="in/l"
         compare "VALUE relative in/l (*.psd)" "$d/proj")
        (export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.excludesFile GIT_CONFIG_VALUE_0=""
         compare "VALUE empty" "$d/proj")
        printf '[core]\n\texcludesFile = %s\n' "$d/home/list" > "$d/home/inc"
        (export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=include.path GIT_CONFIG_VALUE_0="$d/home/inc"
         compare "include.path from a variable" "$d/proj")
        (export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=Include.Path GIT_CONFIG_VALUE_0="$d/home/inc"
         compare "Include.Path from a variable" "$d/proj")
        (export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=include..path GIT_CONFIG_VALUE_0="$d/home/inc"
         compare "include..path from a variable" "$d/proj")
        (export GIT_CONFIG_PARAMETERS="'core.excludesFile'='$d/home/list'"
         compare "GIT_CONFIG_PARAMETERS (git -c)" "$d/proj")
        (export GIT_CONFIG=$d/home/inc; compare "GIT_CONFIG (git config only)" "$d/proj")
        ;;
    values)
        d=$(fresh)
        export HOME=$d/home XDG_CONFIG_HOME=$d/home
        printf '*.key\n' > "$d/home/list"
        mkdir -p "$d/home/dir"
        mkfifo "$d/home/fifo"
        for v in "$d/home/list" '"'"$d/home/list"'"' "$d/home/list " "~/list" "~//list" "~" \
                 "$d/home/dir" "$d/home/fifo" "/dev/null" "/dev//null" "/dev/zero" "$d/home/missing" \
                 ":(optional)$d/home/list" ":(optional)$d/home/missing" "%(prefix)/list"; do
            printf '[core]\n\texcludesFile = %s\n' "$v" > "$d/home/.gitconfig"
            compare "excludesFile = $v" "$d/proj"
        done
        ;;
    esac
}

for s in "${@:-home-unset global system count values}"; do
    for t in $s; do
        echo "== $t"
        scenario "$t"
    done
done
rm -rf "$base"
