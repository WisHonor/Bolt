. /work/redteam/attacks5/lib.sh
# 1: undo of a discard brings the work back; redo throws it away again
fresh s1
echo one > a.txt; b save "One" >/dev/null
echo two > a.txt; echo new > n.txt
b discard; tree
b undo; tree
b undo --list
b undo --list --all
b redo; tree
b status

# 2: shelf, then undo the shelf, then unshelf
fresh s2
echo one > a.txt; b save "One" >/dev/null
echo two > a.txt
b shelf "wip"; tree
b shelf --list
b undo; tree; b shelf --list
b redo; b unshelf; tree; b shelf --list

# 3: amend, then goto the old save by fingerprint and make a line on it
fresh s3
echo one > a.txt; b save "One" >/dev/null
echo two > a.txt; b save "Two" >/dev/null
old=$(b --json status | sed -n 's/.*"save":{"id":"\([0-9a-f]*\)".*/\1/p')
echo "old=$old"
echo three > a.txt; b save --amend "Two amended"
b history
b goto "$old"
echo four > a.txt; b save --line side "Four"
b history
b goto main; b history
