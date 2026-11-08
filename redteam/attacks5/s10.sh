. /work/redteam/attacks5/lib.sh
show "SH1: undo --shelve with an exec flip, a link, a new file in a new folder, a folder made a file"
fresh sh1
echo one > a; echo s > s.sh; mkdir d; echo x > d/x; b save "One" >/dev/null; One=$(cur)
echo two > a; chmod +x s.sh; rm -r d; echo dfile > d; b save "Two" >/dev/null
b goto "$One" >/dev/null 2>&1
# Now the user works over everything the undo would write.
echo mine-a > a; chmod -x s.sh; rm -r d; ln -s a d
b status
b undo --shelve; echo "exit $?"; tree; ls -l s.sh | cut -c1-10
b shelf --list
b unshelf; echo "exit $?"; tree; ls -l s.sh | cut -c1-10
b status

show "SH2: redo --shelve"
fresh sh2
echo one > a; b save "One" >/dev/null; One=$(cur)
echo two > a; mkdir n; echo nn > n/n; b save "Two" >/dev/null
b goto "$One" >/dev/null; b undo >/dev/null
echo mine > a; mkdir -p n/n2; echo deep > n/n2/z
b redo --shelve; echo "exit $?"; tree; b shelf --list
b undo; echo "undo exit $?"; tree; b shelf --list
