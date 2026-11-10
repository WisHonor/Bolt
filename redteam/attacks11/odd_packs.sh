#!/usr/bin/env bash
# RT12: packs that aren't packs (a pipe, a folder, a link, unreadable, empty, a pack of another
# project) named before and after the real one; status, changes and save on this build and on
# main's, each under a 20 s limit, with their outputs side by side.
# Run in the sandbox only:  redteam/attacks11/run.sh script odd_packs.sh
set -u
NEW=/work/target/release/bolt
OLD=/work/_main/target/release/bolt
export BOLT_USER_NAME=r BOLT_USER_EMAIL=r@x
for kind in fifo dir link000 unreadable empty foreign; do
  for where in 0000 ffff; do
    base=$(mktemp -d)
    export BOLT_CONFIG_DIR=$base/cfg
    mkdir -p $base/cfg $base/p/s
    cd $base/p
    $NEW start >/dev/null
    echo a > s/a; echo b > b
    $NEW save one >/dev/null
    echo c > s/c
    packs=.bolt/store/packs
    odd=$packs/$where$(printf 'a%.0s' $(seq 60)).pack
    case $kind in
      fifo) mkfifo $odd ;;
      dir) mkdir $odd ;;
      link000) ln -s /dev/zero $odd ;;
      unreadable) cp $(ls $packs/*.pack | head -1) $odd; chmod 000 $odd ;;
      empty) : > $odd ;;
      foreign) f=$(mktemp -d); (cd $f && BOLT_CONFIG_DIR=$base/cfg $NEW start >/dev/null && echo z > z && $NEW save z >/dev/null); cp $(ls $f/.bolt/store/packs/*.pack | tail -1) $odd ;;
    esac
    for b in NEW OLD; do
      rm -rf $base/$b; cp -a $base/p $base/$b
    done
    for cmd in status changes "save two"; do
      for b in NEW OLD; do
        bin=${!b}
        out=$(cd $base/$b && timeout 20 $bin $cmd 2>&1; echo "exit $?")
        echo "$kind $where [$cmd] $b: $(echo "$out" | sed "s#$base/$b#<root>#g" | tr '\n' ' ' | cut -c1-400)"
      done
    done
    cd /; rm -rf $base
  done
done
