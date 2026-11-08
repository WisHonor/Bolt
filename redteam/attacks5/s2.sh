. /work/redteam/attacks5/lib.sh
show "C: discard a new folder holding an ignored file, then undo"
fresh c
echo '*.o' > .gitignore; echo one > a.txt; b save "One" >/dev/null
mkdir d; echo precious > d/new.txt; echo build > d/x.o
b discard; tree
b undo; tree
b status

show "C2: same with the ignored file removed after the discard"
fresh c2
echo '*.o' > .gitignore; echo one > a.txt; b save "One" >/dev/null
mkdir d; echo precious > d/new.txt; echo build > d/x.o
b discard; rm d/x.o; rmdir d; tree
b undo; tree

show "G: goto removes a folder that holds an ignored file; undo; redo"
fresh g
echo '*.o' > .gitignore; mkdir d; echo f > d/f; b save "A" >/dev/null; A=$(cur)
rm -r d; echo top > t; b save "B" >/dev/null
b goto "$A" ; echo build > d/x.o; tree
b undo; tree
b redo; tree
b status

show "H: discard path, then the user deletes the file, then undo"
fresh h
echo one > a.txt; echo bee > b.txt; b save "One" >/dev/null
echo two > a.txt
b discard a.txt; rm a.txt
b undo; tree; b status

show "I: undo when the user re-made a removed file with the same bytes"
fresh i
echo one > a.txt; b save "One" >/dev/null
echo new > n.txt
b discard; echo new > n.txt
b undo; tree; b undo --list
b redo; tree
