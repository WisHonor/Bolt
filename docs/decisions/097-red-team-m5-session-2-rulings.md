# 097 — Rulings after milestone 5's second red-team session

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 5's red-team gate, [decision 024](024-red-team-gate.md))
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 094](094-red-team-m5-rulings.md) (where the user's Git settings are read, and which long blobs are look-alikes), and with it [decision 047](047-git-personal-ignore-lists-read.md) and [decision 060](060-user-git-settings-includes-and-paths.md) (the home folder those settings are found in)
- **Refined by:** [decision 098](098-a-blob-is-also-judged-in-its-crlf-form.md) (a blob is also judged as Git LFS would read it with CRLF line ends, since a conversion can do more than push bytes out)

## Context

The second red-team session on milestone 5 ([report](../redteam/2026-11-09-milestone-5-session-2.md)) attacked the fixes of decisions 094 to 096. They held for every variable and odd value tried. It proved two findings past them:
- **RT9-1, Medium.** Bolt finds the user's home folder in `HOME`. When `HOME` isn't set, it asks the system: the account's home folder on Unix, the profile folder on Windows. Git does neither.
  - **On Unix,** Git takes the home folder from `HOME` alone. Without it, Git reads no `~/.gitconfig` and no `~/.config/git` files.
  - **On Windows,** `HOME` is usually unset. Git for Windows then sets it to `HOMEDRIVE` plus `HOMEPATH` when that is a folder, else to `USERPROFILE` (`setup_windows_environment` in compat/mingw.c). Where a company gives accounts a home drive, Git's settings live there, and Bolt read another file.

  So saves silently applied a different personal ignore list. In the proof on Windows, a private key Git keeps out went into Bolt's history. And an import refused over an `lfs.storage` Git LFS wasn't using. This is RT8-1 and RT8-2's harm again, through the home folder.
- **RT9-2, Low.** On checkout, Git converts a file before Git LFS's smudge filter reads it. With `eol=crlf`, or `core.autocrlf=true` (which Git for Windows' installer sets), each line end gains a CR. That pushes bytes out of the 1,024 Git LFS reads. The result:
  - a long blob whose own first 1,024 bytes aren't a pointer can be one after conversion. Git LFS users get the pointer's file, while Bolt brings in the blob's text and says nothing, past RT8-3's fix;
  - the other way round, a padded pointer of at most 1,024 bytes can stop being one after conversion. Git LFS users then see its text, while Bolt brings in its file.

  Only a hostile or broken history holds either.

The session also found that decision 094's reason for not reading `-c` options was wrong. They don't reach only Git's own commands: Git passes them to every program it runs (an alias, a hook, Git LFS) in `GIT_CONFIG_PARAMETERS`.

## Options

**For RT9-1: the home folder.**
- **A. Find it as Git does.** On Unix, `HOME` alone. On Windows, `HOME`, else `HOMEDRIVE` plus `HOMEPATH` when that is a folder, else `USERPROFILE`.
- **B. Keep asking the system,** and say so in the docs. That is RT9-1 itself.

**For RT9-2: Git's conversions.**
- **A. Model Git's conversions** (`ident`, line endings, working-tree encoding) per path and commit, from `.gitattributes` and the user's settings. Then judge what Git LFS would read. But the answer differs from machine to machine (`core.autocrlf` is set per user), so there is no one Git LFS view to match.
- **B. Judge the blob's own bytes, as now, but warn more widely.** Name in `lfs_lookalikes` every blob over 1,024 bytes whose first 1,024 bytes *look like* a pointer. That uses the looser test decision 037 gave for look-alikes, not Git LFS's exact parser. A conversion only pushes bytes out, so such a blob still looks like a pointer. Leave the other way round as a known limit: Bolt holds the pointer's real file, checked against its SHA-256.
- **C. Leave it.**

## Decision

**A for RT9-1, B for RT9-2.**

**RT9-1.** The home folder in which Bolt finds the user's Git settings is found as Git v2.56.0 finds it, in one place:
- **On Unix:** `HOME` alone, an empty value taken as Git takes it.
- **On Windows:** `HOME`; else `HOMEDRIVE` followed by `HOMEPATH` when that names a folder; else `USERPROFILE`.
- **With no home folder,** Bolt reads no `~/.gitconfig` and no `$HOME/.config/git` files. It still reads `XDG_CONFIG_HOME`'s files when that is set, and the files and settings the `GIT_CONFIG_*` variables give, as Git does. A setting that starts with `~/` then stops the command and names it, as Git stops ("failed to expand user dir").

Bolt's own settings folder is unchanged.

**RT9-2.** A blob over 1,024 bytes whose first 1,024 bytes look like a Git LFS pointer, by the test decision 037 gave, comes in as its own bytes. It is named in `lfs_lookalikes`, as decision 094 says for one Git LFS's parser accepts. The other way round (a padded pointer that Git's conversions push past 1,024 bytes) stays a known limit, beside the deferred row on judging Git LFS files by `.gitattributes`.

**`-c` options.** Decision 094's reason was wrong. They reach programs Git runs, in `GIT_CONFIG_PARAMETERS`, and Bolt doesn't read that variable yet. A follow-up row reads it in Git's quoted format. Until then, a setting given with `git -c` to an alias that runs Bolt doesn't reach Bolt.

## Consequences

- One place finds the home folder for Git's settings, in crates/bolt-vcs-core/src/config.rs. It has unit tests for each system's rule, with the environment passed in.
- The proof tests in `crates/bolt-vcs/tests/redteam9.rs` are no longer ignored. `rt9_1` must never write the real home folder: it proves the rule with a run that has no `HOME` and with the lookup's own tests. The Windows test from the red team's branch (`redteam9_windows.rs`) joins the suite with the fix.
- storage-format.md §12, json.md (`lfs_lookalikes`), design.md §5.16, security.md T31 and the follow-ups describe the change and the limits.
