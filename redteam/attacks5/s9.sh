. /work/redteam/attacks5/lib.sh
# A discard stopped partway (after its first file), then state/head lost: what can the user do?
fresh j0
echo one > a.txt; echo bee > b.txt; b save "One" >/dev/null
echo PRECIOUS-A > a.txt; echo PRECIOUS-B > b.txt; echo NEW > c.txt
for n in $(seq 1 40); do
  rm -rf /tmp/rt/j; cp -a /tmp/rt/j0 /tmp/rt/j; cd /tmp/rt/j
  out=$(BOLT_FAULT_AT=$n "$BOLT" discard 2>&1)
  echo "$n: $(echo "$out" | grep -o 'injected crash at [a-z:-]*')"
  if echo "$out" | grep -q 'injected crash at switch:changed'; then break; fi
done
tree; ls .bolt/state
rm .bolt/state/head
b undo --list
e=$(b --json undo --list | grep -o '"entry":"[0-9a-f]*"' | head -1 | cut -d'"' -f4)
echo "picking $e"
b undo --to "$e"; echo "exit $?"
b status; echo "status exit $?"
b undo --list --all
b discard; echo "discard exit $?"
b undo; echo "undo exit $?"
tree
