. /work/redteam/attacks5/lib.sh
E=$(printf '\033[31mRED\033[0m')
BIDI=$(printf 'abc\342\200\256cba')
NL=$(printf 'line1\nline2')
CR=$(printf 'fake\rOVERWRITE')
show "escapes in shelf messages, save messages, amend messages, file names"
fresh t
echo one > a.txt; b save "msg $E $BIDI" >/dev/null
echo two > a.txt; b shelf "shelf $E $BIDI $CR" | od -c | grep -n '033\|342\|\\r' | head
echo "--- shelf --list"; b shelf --list | od -c | grep -c '033'
echo three > "$E.txt"; b save "x$NL" 2>&1 | od -c | grep -c '033'
echo "--- undo --list"; b undo --list | od -c | grep '033\|342 200 256\|\\r' | head
echo "--- undo --list --all"; b undo --list --all | od -c | grep '033\|342 200 256\|\\r' | head
echo "--- undo"; b undo 2>&1 | od -c | grep '033\|342 200 256\|\\r' | head
echo "--- redo"; b redo 2>&1 | od -c | grep '033\|342 200 256\|\\r' | head
echo four > a.txt; b save --amend "am $E $CR" 2>&1 | od -c | grep '033\|\\r' | head
echo "--- undo amend"; b undo 2>&1 | od -c | grep '033\|342 200 256\|\\r' | head
echo "--- unshelf"; b unshelf 2>&1 | od -c | grep '033\|342 200 256\|\\r' | head
b undo --list --all | cat -v
b shelf --list | cat -v
