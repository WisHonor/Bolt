. /work/redteam/attacks5/lib.sh
mkdir -p /tmp/rt; echo OUTSIDE > /tmp/rt/outside.txt
show "L1: discard a new link, undo, redo"
fresh l1
echo one > a.txt; b save "One" >/dev/null
ln -s a.txt l; b discard; tree; b undo; tree; b redo; tree

show "L2: a saved file replaced by a link; discard; undo"
fresh l2
echo one > a.txt; echo f > f; b save "One" >/dev/null
rm f; ln -s a.txt f; b discard; tree; b undo; tree; b status

show "L4: a saved folder replaced by a link to another folder of the project; goto that changes d/x"
fresh l4
mkdir d d2; echo x1 > d/x; echo theirs > d2/x; b save "A" >/dev/null; A=$(cur)
echo x2 > d/x; b save "B" >/dev/null
rm -r d; ln -s d2 d; b status
b goto "$A" 2>&1; tree
b discard 2>&1; tree; b undo 2>&1; tree

show "L5: a save has link d -> d2; goto it, then undo with a file of the user's in d2"
fresh l5
mkdir d2; echo theirs > d2/x; b save "A" >/dev/null; A=$(cur)
ln -s d2 d; b save "B" >/dev/null
b goto "$A"; tree; b redo; tree; echo mine > d2/y; b undo; tree; b redo; tree

show "L6: placeholder for a link leading out; user turns the placeholder into their notes; goto/undo"
fresh l6
echo a > a; b save "A" >/dev/null; A=$(cur)
ln -s ../outside.txt out; b save "B" >/dev/null; B=$(cur)
b goto "$A"; b goto main; tree
b undo; tree; b redo; tree
printf '../outside.txt' > out2; chmod +x out; tree
b status
b goto "$A"; tree; b status
b undo; tree
