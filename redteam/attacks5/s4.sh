. /work/redteam/attacks5/lib.sh
show "T1: goto between file d and folder d, undo, redo"
fresh t1
echo filed > d; echo k > k; b save "A file d" >/dev/null; A=$(cur)
rm d; mkdir d; echo inside > d/x; b save "B folder d" >/dev/null
b goto "$A"; tree; b undo; tree; b redo; tree; b undo; tree; b status

show "T2: discard a folder the user made in place of a saved file, undo, redo"
fresh t2
echo filed > d; b save "A" >/dev/null
rm d; mkdir d; echo mine > d/new; echo mine2 > d/new2
b discard; tree; b undo; tree; b redo; tree; b undo; tree; b status

show "T3: discard a file the user made in place of a saved folder, undo"
fresh t3
mkdir d; echo inside > d/x; echo y > d/y; b save "A" >/dev/null
rm -r d; echo myfile > d
b discard; tree; b undo; tree; b status

show "T4: shelf a file over a saved folder, unshelf onto a save where the folder changed elsewhere"
fresh t4
mkdir d; echo inside > d/x; echo o > o; b save "A" >/dev/null
rm -r d; echo myfile > d
b shelf; tree
echo o2 > o; b save "B" >/dev/null
b unshelf; tree; b status
b undo; tree; b status; b shelf --list

show "T5: discard d/x only, where d is now a file of the user's"
fresh t5
mkdir d; echo inside > d/x; b save "A" >/dev/null
rm -r d; echo myfile > d
b discard d/x; tree; b status
b discard d; tree; b undo; tree

show "T6: a folder becomes a file in a goto; user adds a new file in the folder after undo; redo"
fresh t6
mkdir d; echo inside > d/x; b save "A" >/dev/null; A=$(cur)
rm -r d; echo filed > d; b save "B" >/dev/null
b goto "$A"; tree; echo mine > d/mine; b redo 2>&1 | head -3; b undo; tree; b redo; tree; b status
