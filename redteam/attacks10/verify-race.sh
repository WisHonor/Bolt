#!/usr/bin/env bash
# Runs `bolt verify` over and over while another loop saves, undoes, goes to other saves and
# tidies packs it has just aged, so that every tidy rewrites them: any problem verify reports is
# a false alarm, since every step is Bolt's own. Sandbox only.
#   verify-race.sh [seconds]
set -u
BOLT=${BOLT:-/work/target/debug/bolt}
SECS=${1:-180}
export BOLT_CONFIG_DIR=$(mktemp -d)
printf 'undo-window-days = 1\n[user]\nname = "T"\nemail = "t@example.invalid"\n' > "$BOLT_CONFIG_DIR/config.toml"
p=$(mktemp -d); cd "$p"
echo a > a.txt; head -c 1500000 /dev/urandom > big.bin
$BOLT start --preset custom >/dev/null 2>&1
end=$(( $(date +%s) + SECS ))
(
    i=0
    while [ "$(date +%s)" -lt "$end" ]; do
        i=$((i + 1))
        echo "$i" >> a.txt
        [ $((i % 3)) -eq 0 ] && head -c 1500000 /dev/urandom > "big$((i % 4)).bin"
        $BOLT save "s$i" --yes >/dev/null 2>&1
        [ $((i % 4)) -eq 1 ] && $BOLT undo >/dev/null 2>&1
        [ $((i % 5)) -eq 2 ] && { echo x >> a.txt; $BOLT shelf "sh$i" >/dev/null 2>&1; }
        [ $((i % 7)) -eq 3 ] && $BOLT goto main >/dev/null 2>&1
        find .bolt/store/packs -name '*.pack' -exec touch -d '3 days ago' {} +
        $BOLT tidy >/dev/null 2>&1
    done
    echo "changer: $i rounds"
) &
runs=0; alarms=0
while [ "$(date +%s)" -lt "$end" ]; do
    out=$($BOLT verify 2>&1); code=$?
    runs=$((runs + 1))
    if [ $code -ne 0 ]; then
        alarms=$((alarms + 1))
        [ $alarms -le 5 ] && { echo "--- verify run $runs exited $code"; echo "$out" | head -12; }
    fi
done
wait
echo "verify: $runs runs, $alarms with problems"
$BOLT verify >/dev/null 2>&1 && echo "final verify: clean" || { echo "final verify:"; $BOLT verify 2>&1 | head; }
