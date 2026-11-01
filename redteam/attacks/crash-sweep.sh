#!/bin/sh
# Crash at every fault point of `bolt save` and `bolt start`, then check the project still works.
# Run inside the sandbox after: CARGO_TARGET_DIR=/tmp/tf cargo build -q -p bolt-vcs --features fault-injection
B=/tmp/tf/debug/bolt
export BOLT_USER_NAME=T BOLT_USER_EMAIL=t@e
for n in 1 2 3 4 5 6 7 8 9 10 11 12; do
  D=$(mktemp -d); cd "$D"; export BOLT_CONFIG_DIR=$D
  $B start >/dev/null || echo "n=$n start failed"
  echo one > a; $B save one >/dev/null
  echo two > a; echo new > b
  BOLT_FAULT_AT=$n $B save two >/dev/null 2>&1; c1=$?
  $B history >h.txt 2>&1; c2=$?
  $B save again >s.txt 2>&1; c3=$?
  $B show --json >j.txt 2>&1; c4=$?
  left=$(ls .bolt/tmp | wc -l)
  echo "n=$n crashed_save=$c1 history=$c2 save_after=$c3 show=$c4 tmp_left=$left $(head -c 120 s.txt)"
done
for n in 1 2 3 4 5 6 7 8; do
  D=$(mktemp -d); cd "$D"; export BOLT_CONFIG_DIR=$D
  BOLT_FAULT_AT=$n $B start >/dev/null 2>&1; c1=$?
  $B start >o.txt 2>&1; c2=$?
  $B history >/dev/null 2>&1; c3=$?
  echo "start n=$n crashed=$c1 restart=$c2 history=$c3 $(head -c 100 o.txt)"
done
