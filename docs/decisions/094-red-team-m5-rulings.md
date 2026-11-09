# 094 — Rulings after milestone 5's red-team session

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 5's red-team gate, [decision 024](024-red-team-gate.md))
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 047](047-git-personal-ignore-lists-read.md) and [decision 060](060-user-git-settings-includes-and-paths.md) (where the user's own Git settings are read), [decision 089](089-git-lfs-objects-kept-elsewhere.md) (where `lfs.storage` is read, and what the stop for missing Git LFS files says) and [decision 085](085-importing-git-lfs-files.md) (what counts as a look-alike)
- **Refined by:** [decision 095](095-a-git-setting-naming-dev-null-names-no-file.md) (`/dev/null`, and `nul` on Windows, names no file, judged before it is joined to a folder)

## Context

The first red-team session on milestone 5 ([report](../redteam/2026-11-09-milestone-5.md)) proved three findings. The big-file store itself held. All three are places where Bolt reads Git's world differently from Git.

- **RT8-1, Medium.** Bolt reads `lfs.storage` from the user's own Git settings (`$XDG_CONFIG_HOME/git/config` and `~/.gitconfig`, with plain includes), `.git/config` and `config.worktree`. Git, and Git LFS through it, read more:
  - the system's Git settings;
  - the file `GIT_CONFIG_GLOBAL` names, *in place of* the two home files;
  - settings given through `GIT_CONFIG_COUNT`;
  - `[includeIf]` sections.

  So when `lfs.storage` is set where Bolt doesn't look, the import says the files are missing and to run `git lfs fetch --all`, which can't help. And under `GIT_CONFIG_GLOBAL`, Bolt still reads `~/.gitconfig`, and refuses an import over a setting Git isn't using.
- **RT8-2, Medium.** The same reader finds `core.excludesFile`, the user's own ignore list (decision 047). Under `GIT_CONFIG_GLOBAL`, Bolt applies the list `~/.gitconfig` names instead of the one Git applies. Saves then silently leave out files Git tracks, and take in files the user's real list keeps out, such as keys.
- **RT8-3, Low.** Git LFS's smudge filter reads a blob's first 1,024 bytes and, if they parse as a pointer, checks out the pointer's file, whatever follows. Bolt counts as a Git LFS file only a blob of at most 1,024 bytes (decision 085). So a longer blob that starts with a pointer comes into Bolt as its text, tail and all, while Git LFS shows the pointer's file, and nothing warns. Git LFS never writes such a blob: it takes a hostile or broken history.

## Options

**For RT8-1 and RT8-2: where the user's settings are read.**
- **A. Read them where Git does, as far as Bolt can know without running Git.** That means `GIT_CONFIG_SYSTEM`, `GIT_CONFIG_NOSYSTEM`, `GIT_CONFIG_GLOBAL`, and `GIT_CONFIG_COUNT` with its keys and values. Where Bolt still can't know, its messages say so.
- **B. A, and also guess where the system's own file is,** from the usual place or from the `git` found on the PATH. But that place depends on how Git was built: `/etc/gitconfig` for most Linux packages, `/opt/homebrew/etc/gitconfig` for Homebrew, and the install folder's `etc\gitconfig` for Git for Windows. A guess can read a file Git doesn't, which is RT8-1's own mismatch the other way round.
- **C. Ask Git:** run `git config` and read its answer. That is exact, but Bolt runs no programs. A hostile repository's settings can make Git run commands (T6), and an import would need Git installed.

**For RT8-3: a blob over 1,024 bytes that starts with a pointer.**
- **A. Keep its text, as a look-alike, and name it in the `lfs_lookalikes` warning.**
- **B. Bring in the file Git LFS checks out.** But Git LFS's own scanner, which `git lfs ls-files` and `git lfs fetch` use, skips blobs over 1,024 bytes. So that file may never have been fetched, and Git LFS disagrees with itself about the blob.
- **C. Leave it:** only a hostile history holds one.

## Decision

**A and A.**

**RT8-1 and RT8-2.** Bolt takes two settings from the user's Git settings: `core.excludesFile` (decision 047) and `lfs.storage` (decision 089). It now reads them in Git's order, the last one found winning:
1. **The system's file,** when `GIT_CONFIG_SYSTEM` names one and `GIT_CONFIG_NOSYSTEM` isn't true.
2. **The user's file:** the one `GIT_CONFIG_GLOBAL` names, when it is set, in place of the two home files. Otherwise `$XDG_CONFIG_HOME/git/config`, then `~/.gitconfig`, as before.
3. **For `lfs.storage` only:** the repository's `.git/config`, then `config.worktree` when `extensions.worktreeConfig` is set (decision 089). Bolt never takes `core.excludesFile` from the repository.
4. **The settings `GIT_CONFIG_COUNT` gives,** through `GIT_CONFIG_KEY_<n>` and `GIT_CONFIG_VALUE_<n>`, after every file, as Git applies them. They are the user's own, like the files in 1 and 2.

More details:
- Each file's plain includes are followed as decision 060 says.
- Git v2.56.0's own rules are the reference for these variables, including an empty value. A value Git would refuse stops the command and names the variable.

**What Bolt still doesn't read:**
- the system's own file when `GIT_CONFIG_SYSTEM` isn't set;
- `[includeIf]` sections (decision 060);
- `-c` options, which only reach Git's own commands.

Where those can matter, Bolt says so:
- **Missing Git LFS files.** The stop also says that a Git setting Bolt doesn't read (the system's Git settings, or an `[includeIf]` section) may keep Git LFS's files elsewhere. It says that `git lfs env` shows where they are kept, on its `LocalMediaDir` line, and that if that isn't `.git/lfs/objects`, the way out is the one decision 089's stop gives for files kept elsewhere.
- **The personal ignore list.** storage-format.md §12 names the places Bolt doesn't read.

**RT8-3.** A blob over 1,024 bytes whose first 1,024 bytes Git LFS's smudge filter takes as a pointer comes in as its own bytes, as a look-alike does. It is counted and named in the `lfs_lookalikes` warning (decision 085).
- The test is Git LFS's own: the bytes its `DecodeFrom` reads, then the parser `lfs_pointer` already follows.
- A Git LFS file is still a blob of at most 1,024 bytes.
- The check reads at most a blob's first 1,024 bytes, while the import reads that blob anyway.

## Consequences

- One place finds the user's Git settings for both settings: `crates/bolt-vcs-core/src/config.rs`.
- The proof tests in `crates/bolt-vcs/tests/redteam8.rs` are no longer ignored:
  - RT8-2 and RT8-3 run as the red team wrote them;
  - RT8-1 checks the import under `GIT_CONFIG_GLOBAL` and `GIT_CONFIG_SYSTEM`. For `[includeIf]` it checks the new words in the stop.
- storage-format.md §12, json.md (the stop for missing Git LFS files and `lfs_lookalikes`), design.md §5.16 and security.md describe the change.
- The session's "Not proven" items join the follow-ups.
