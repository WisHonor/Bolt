. /work/redteam/attacks5/lib.sh
show "H1: head lost; list; pick the newest; status"
fresh h1
echo one > a.txt; b save "One" >/dev/null
echo two > a.txt; b save "Two" >/dev/null
cp .bolt/state/head /tmp/rt/h1.head
rm .bolt/state/head
b status; echo "exit $?"
b undo --list; echo "exit $?"
b undo --list --all; echo "exit $?"
b --json undo --list; echo
b undo; echo "exit $?"
e=$(b undo --list | awk 'NR==1{print $1}'); echo "pick $e"
b undo --to "$e"; echo "exit $?"
b status; b undo --list
show "H2: head garbled: uppercase, CRLF, extra bytes, empty, directory"
for v in upper crlf empty dir big; do
  cp /tmp/rt/h1.head .bolt/state/head 2>/dev/null
  case $v in
    upper) tr a-f A-F < /tmp/rt/h1.head > .bolt/state/head;;
    crlf) sed 's/$/\r/' /tmp/rt/h1.head > .bolt/state/head;;
    empty) : > .bolt/state/head;;
    dir) rm .bolt/state/head; mkdir .bolt/state/head;;
    big) head -c 100000000 /dev/zero > .bolt/state/head;;
  esac
  echo "[$v]"; timeout 20 "$BOLT" status 2>&1 | head -3; echo "exit ${PIPESTATUS[0]}"
  rm -rf .bolt/state/head
done
cp /tmp/rt/h1.head .bolt/state/head
show "H3: head names a save, not an entry"
cur > /dev/null; s=$(cur); echo "$s" > .bolt/state/head; b status 2>&1 | head -3
b undo --list 2>&1 | head
