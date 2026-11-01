#!/bin/sh
# RT-4: state/head -> /dev/zero. Run inside the sandbox: sh redteam/attacks/rt4-dev-zero-head.sh
# Shows how much memory `bolt history` takes before it stops (no ulimit; container has 4 GB).
BOLT=${BOLT:-/tmp/t/debug/bolt}
D=$(mktemp -d); cd "$D" || exit 1
export BOLT_USER_NAME=T BOLT_USER_EMAIL=t@e BOLT_CONFIG_DIR=$D
$BOLT start >/dev/null
rm .bolt/state/head && ln -s /dev/zero .bolt/state/head
start=$(date +%s)
$BOLT history >out.txt 2>err.txt &
pid=$!
peak=0
while [ -d /proc/$pid ] && ! grep -q '^State:.*Z' /proc/$pid/status 2>/dev/null; do
  r=$(awk '/VmRSS/{print $2}' /proc/$pid/status 2>/dev/null)
  [ -n "$r" ] && [ "$r" -gt "$peak" ] && peak=$r
  sleep 0.1
done
wait $pid; code=$?
echo "exit=$code  peak RSS ${peak} kB  after $(( $(date +%s) - start )) s"
echo "stderr: $(cat err.txt)"
