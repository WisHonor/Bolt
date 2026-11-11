#!/usr/bin/env bash
# RT12: packs merged and removed under a reader. One process saves and tidies in a loop (every
# save adds a pack; small packs merge after a commit once more than 32 are small, and tidy
# rewrites what it keeps), while another runs `status` and `changes` over and over. Every
# reader run that exits non-zero or warns is counted and its words kept; this build, then main's.
# Run in the sandbox only:  redteam/attacks11/run.sh script concurrent_tidy.sh [rounds]
set -u
ROUNDS=${1:-60}
export BOLT_USER_NAME=r BOLT_USER_EMAIL=r@x
for which in NEW OLD; do
  case $which in NEW) B=/work/target/release/bolt ;; OLD) B=/work/_main/target/release/bolt ;; esac
  base=$(mktemp -d); export BOLT_CONFIG_DIR=$base/cfg; mkdir -p $base/cfg $base/p
  cd $base/p
  for i in $(seq 1 300); do mkdir -p d$((i % 30))/e$((i % 7)); echo "$i" > d$((i % 30))/e$((i % 7))/f$i; done
  $B start >/dev/null; $B save first >/dev/null
  (
    for r in $(seq 1 $ROUNDS); do
      echo "$r" >> d$((r % 30))/e$((r % 7))/f$r 2>/dev/null || echo "$r" > d1/x$r
      $B save "r$r" >/dev/null 2>&1
      if [ $((r % 10)) -eq 0 ]; then $B tidy >/dev/null 2>&1; fi
    done
    touch $base/writer-done
  ) &
  bad=0; runs=0
  : > $base/said
  while [ ! -e $base/writer-done ]; do
    for cmd in status changes; do
      out=$($B $cmd 2>&1 >/dev/null); code=$?
      runs=$((runs + 1))
      if [ $code -ne 0 ] || [ -n "$out" ]; then
        bad=$((bad + 1)); echo "[$cmd $code] $out" | head -c 300 >> $base/said; echo >> $base/said
      fi
    done
  done
  wait
  echo "$which: $runs reader runs, $bad exited non-zero or warned; packs now $(ls .bolt/store/packs | wc -l)"
  sort $base/said | uniq -c | sort -rn | head -8 | sed "s#$base#<base>#g"
  cd /; rm -rf $base
done
