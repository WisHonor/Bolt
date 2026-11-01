#!/bin/sh
# Probes of `bolt save` on awkward working folders (RT-11 and the "held up" list).
# Run inside the sandbox: sh redteam/attacks/save-probes.sh
B=${BOLT:-/tmp/t/debug/bolt}
export BOLT_USER_NAME=T BOLT_USER_EMAIL=t@e
D=$(mktemp -d); cd "$D"; export BOLT_CONFIG_DIR=$D; $B start >/dev/null
echo "--- unreadable folder"
mkdir locked; echo s > locked/f; chmod 000 locked; $B save m1; echo "exit=$?"; chmod 755 locked; rm -r locked
echo "--- multi-line error (name with a newline)"
printf x > "$(printf 'new\nline')"; $B save m2; echo "exit=$?"; rm -f "$(printf 'new\nline')"
echo "--- nested .bolt folder, FIFO, self-hiding .boltignore"
mkdir -p sub/.bolt v; echo data > sub/.bolt/notes.txt; mkfifo pipe
printf '*\n' > v/.boltignore; echo keep > v/important.c; echo top > top.txt
$B save m3; echo "exit=$?"; $B show
