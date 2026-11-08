. /work/redteam/attacks5/lib.sh
show "IG1: undo of a discard writes a file where an ignored file of the user's now is"
fresh ig1
echo one > a.txt; b save "One" >/dev/null
echo first > notes.log; b discard >/dev/null
echo '*.log' > .gitignore; echo precious-ignored > notes.log
b undo; echo "exit $?"; tree

show "IG2: undo of a goto writes build/x where an ignored build/x now is"
fresh ig2
echo 'build/' > .gitignore.off; mkdir build; echo tracked > build/x; echo a > a; b save "A" >/dev/null; A=$(cur)
rm -r build; b save "B" >/dev/null
mv .gitignore.off .gitignore; b save "B2" >/dev/null
b goto "$A" 2>&1; tree
b undo; b undo 2>&1; tree

show "IG3: redo of a goto onto an ignored file under a folder the goto makes"
fresh ig3
echo '*.o' > .gitignore; echo a > a; b save "A" >/dev/null; A=$(cur)
mkdir d; echo tracked > d/x.o.txt; echo f > d/f.o; b save "B" 2>&1; B=$(cur); b status
cat .gitignore
b goto "$A"; tree; b undo; tree
show "IG4: undo of a step removes a folder; user put an ignored file and a new unignored file there"
fresh ig4
echo '*.o' > .gitignore; echo a > a; b save "A" >/dev/null; A=$(cur)
mkdir d; echo t > d/t; b save "B" >/dev/null
b goto "$A"; b undo; tree
echo obj > d/x.o; echo mine > d/mine.txt
b redo; echo "exit $?"; tree; b status
