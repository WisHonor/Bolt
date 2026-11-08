. /work/redteam/attacks5/lib.sh
# Each command with stdout or stderr closed, full, or a reader gone; exit code and a panic check.
setup() {
  fresh o
  echo one > a.txt; b save "One" >/dev/null
  for i in $(seq 1 30); do echo "$i" > a.txt; b save "S$i" >/dev/null; done
  echo two > a.txt; b shelf "wip" >/dev/null; b undo >/dev/null; b redo >/dev/null; echo 2 > a.txt; b discard >/dev/null; b undo >/dev/null; b discard >/dev/null
  b goto main >/dev/null 2>&1
}
run() { # $1 label, rest: command
  local label=$1; shift
  for mode in closed_out full_out pipe_out closed_err full_err both_full; do
    setup >/dev/null 2>&1
    case $mode in
      closed_out) "$BOLT" "$@" >&- 2>/tmp/rt/err; c=$?;;
      full_out) "$BOLT" "$@" >/dev/full 2>/tmp/rt/err; c=$?;;
      pipe_out) "$BOLT" "$@" 2>/tmp/rt/err | head -c 1 >/dev/null; c=${PIPESTATUS[0]};;
      closed_err) "$BOLT" "$@" >/dev/null 2>&-; c=$?; : > /tmp/rt/err;;
      full_err) "$BOLT" "$@" >/dev/null 2>/dev/full; c=$?; : > /tmp/rt/err;;
      both_full) "$BOLT" "$@" >/dev/full 2>/dev/full; c=$?; : > /tmp/rt/err;;
    esac
    p=$(grep -c panick /tmp/rt/err)
    h=$(b --json status 2>/dev/null | grep -o '"save":{[^}]*' | head -c 0)
    last=$(b undo --list --all 2>/dev/null | head -1 | cut -c1-60)
    echo "$label $mode exit=$c panic=$p last: $last"
  done
}
run undo undo
run redo redo
run list undo --list --all
run unshelf unshelf
run shelf-list shelf --list
run tidy tidy
run amend save --amend "Amended"
run jsonundo --json undo
