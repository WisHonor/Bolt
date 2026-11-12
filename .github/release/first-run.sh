#!/usr/bin/env bash
# A new person's first run on Linux or macOS: the download page, then the guide inside the
# archive, word for word. Each line taken from a document is checked to be still in it
# before it runs. Changes this computer's home folder: run it only on a throwaway runner.
# Usage: first-run.sh <folder with the archive and SHA256SUMS> <version> <target> <download-page.md>
set -eEuo pipefail
[ "${GITHUB_ACTIONS:-}" = true ] || { echo "first-run.sh removes Opalro and its settings from this account and edits ~/.bashrc or ~/.zshrc: it runs only in CI" >&2; exit 2; }

from=$1 version=$2 target=$3 page=$4
archive="opalro-$version-$target.tar.gz"
current=""
trap 'echo "first-run FAILED at: $current" >&2' ERR
step() { current=$1; echo; echo "== $1"; }
# The line must be in the document; then it runs as written.
in_doc() { grep -qF -- "$2" "$1" || { echo "no longer in $1: $2" >&2; false; }; }
run() { in_doc "$guide" "$1"; echo "\$ $1"; eval "$1"; }

if [ "$(uname)" = Darwin ]; then
  rc=.zshrc shell=zsh config="$HOME/Library/Application Support/opalro"
else
  rc=.bashrc shell=bash config="$HOME/.config/opalro"
fi
unset OPALRO_CONFIG_DIR XDG_CONFIG_HOME OPALRO_USER_NAME OPALRO_USER_EMAIL
# What a new terminal reads: the system's own folders, then the shell's start-up file.
new_shell() { env -i HOME="$HOME" TERM=dumb PATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin "$shell" -i -c "$1" 2> /dev/null; }

step "the download page: the archive for this system, and SHA256SUMS, into the same folder"
in_doc "$page" "| \`$archive\` |"
in_doc "$page" "Download the archive for your system, and \`SHA256SUMS\`, into the same folder."
downloads="$HOME/Downloads/opalro-first-run"
mkdir -p "$downloads"
cp "$from/$archive" "$from/SHA256SUMS" "$downloads/"
cd "$downloads"
# The guide is read where the download page says it is published; it is the archive's.
guide="$(mktemp -d)/guide.md"
tar -xzOf "$archive" docs/guide.md > "$guide"
in_doc "$page" "follow the user guide's section 2, Installing"

step "the guide: check the download"
if [ "$(uname)" = Darwin ]; then
  line="grep opalro-$version-aarch64-apple-darwin.tar.gz SHA256SUMS | shasum -a 256 -c"
  in_doc "$guide" "for an Intel Mac, use \`opalro-$version-x86_64-apple-darwin.tar.gz\` in its place"
else
  line="grep opalro-$version-x86_64-unknown-linux-gnu.tar.gz SHA256SUMS | sha256sum -c"
fi
in_doc "$guide" "$line"
said=$(eval "${line/opalro-$version-aarch64-apple-darwin.tar.gz/$archive}")
echo "$said"
in_doc "$guide" "It prints the file's name and \`OK\`"
[ "$said" = "$archive: OK" ]

step "the guide: unpack, and move opalro to ~/.local/bin"
in_doc "$guide" "(Use your archive's name in the first line.)"
line="tar -xzf opalro-$version-x86_64-unknown-linux-gnu.tar.gz"
in_doc "$guide" "$line"
echo "\$ tar -xzf $archive"
tar -xzf "$archive"
in_doc "$page" "Read the user agreement, \`USER-AGREEMENT.md\`."
[ -f USER-AGREEMENT.md ]
run "mkdir -p ~/.local/bin"
run "mv opalro ~/.local/bin/"

step "the guide: add ~/.local/bin to the PATH in ~/$rc"
# Not found before, so the guide's line is what puts it there.
if new_shell "command -v opalro"; then echo "opalro is found before the PATH line is added" >&2; false; fi
line="echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zshrc"
in_doc "$guide" "$line"
in_doc "$guide" "on Linux, write \`~/.bashrc\` in its place"
echo "\$ ${line/.zshrc/$rc}"
eval "${line/.zshrc/$rc}"

step "the guide: in a new terminal, opalro --version"
in_doc "$guide" "Check that it works, in a new terminal:"
found=$(new_shell "command -v opalro")
echo "found: $found"
[ "$found" = "$HOME/.local/bin/opalro" ]
said=$(new_shell "opalro --version")
echo "$said"
[ "$said" = "opalro $version" ]
in_doc "$guide" "opalro $version"
# The rest runs as that new terminal would.
PATH=$(new_shell 'printf %s "$PATH"')
export PATH

step "the guide: tell Opalro who you are"
if [ "$(uname)" = Darwin ]; then
  run "mkdir -p ~/Library/Application\ Support/opalro"
  run "touch ~/Library/Application\ Support/opalro/config.toml"
  in_doc "$guide" "open -e ~/Library/Application\ Support/opalro/config.toml"
else
  run "mkdir -p ~/.config/opalro"
  in_doc "$guide" "nano ~/.config/opalro/config.toml"
fi
# The editor's work: the guide's three lines, saved.
for l in '[user]' 'name = "Ana Lima"' 'email = "ana@example.com"'; do
  in_doc "$guide" "$l"
  echo "$l"
done > "$config/config.toml"
cat "$config/config.toml"

step "the guide: the first five minutes, in a new folder"
mkdir "$HOME/practice"
cd "$HOME/practice"
# The ten lines below are every line of the section's sh blocks; one added there fails here.
count=$(awk '/^## 3\./ { s = 1 } /^## 4\./ { s = 0 } s && /^```/ { b = b ? 0 : ($0 == "```sh"); next } s && b' "$guide" | wc -l | tr -d ' ')
[ "$count" = 10 ]
run 'opalro start --preset code'
run 'echo "hello" > notes.txt'
run 'opalro save "First notes"'
run 'opalro status'
run 'echo "hello again" > notes.txt'
run 'opalro status'
run 'opalro changes'
run 'opalro save "More notes"'
run 'opalro history'
run 'opalro undo'
[ "$(cat notes.txt)" = "hello again" ]

step "the guide: removing Opalro from a computer"
cd "$HOME"
run "rm ~/.local/bin/opalro"
if [ "$(uname)" = Darwin ]; then
  run "rm -r ~/Library/Application\ Support/opalro"
else
  run "rm -r ~/.config/opalro"
fi
[ ! -e "$config" ]
in_doc "$guide" "Then take \`~/.local/bin\` off your PATH by deleting the \`export PATH=\"\$HOME/.local/bin:\$PATH\"\` line from \`~/.zshrc\` or \`~/.bashrc\`, but only if you added that line yourself for Opalro and no other program lives in that folder."
if [ -z "$(ls -A ~/.local/bin)" ]; then
  # The editor's work: that one line deleted.
  grep -vxF 'export PATH="$HOME/.local/bin:$PATH"' "$HOME/$rc" > "$HOME/$rc.new" || [ $? = 1 ]
  mv "$HOME/$rc.new" "$HOME/$rc"
  gone=yes
else
  echo "Other programs live in ~/.local/bin, so the line stays."
  gone=no
fi

step "in a new terminal, opalro is no longer found"
newpath=$(new_shell 'printf %s "$PATH"')
echo "PATH: $newpath"
if [ "$gone" = yes ]; then
  case ":$newpath:" in *":$HOME/.local/bin:"*) echo "~/.local/bin is still on the PATH" >&2; false ;; esac
fi
if new_shell "command -v opalro"; then false; fi

rm -rf "$downloads" "$HOME/practice" "$(dirname "$guide")"
echo
echo "first-run passed: $target"
