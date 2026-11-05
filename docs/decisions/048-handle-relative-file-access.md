# 048 — Bolt holds folders open and works inside them, never through a link

- **Status:** Accepted
- **Date:** 2026-11-05
- **Agenda topic:** none (milestone 3 planning, question 1)
- **Decided by:** the owner.
- **Refines:** [decision 022](022-engineering-rules.md) (`unsafe` only in the platform crate) and [decision 011](011-status-and-write-out.md) (write-out never goes through a link)

## Context

Milestone 3 writes files into the project folder, and `status` and `save` walk it. Rust's standard library works by path: each call looks the path up again, part by part, and follows any link on the way. If something swaps a folder for a link between two calls, Bolt reads or writes where the link points: outside the project, or into `.git` or `.bolt` (security.md T3, T19, T20). Git, Mercurial and Jujutsu work by path too, checking each part for a link first, which leaves a window between the check and the use.

The safe way is to hold each folder open (a handle) and open, make, rename and remove what is in it relative to that handle, refusing a link at every step: `openat` with `O_NOFOLLOW` on Unix, `NtCreateFile` with a root folder handle on Windows.

## Options

- **A. Write it all ourselves** in the platform crate: about 500 lines or more of `unsafe` system calls.
- **B. The `cap-std` crate.** Its no-follow option covers only the last part of a path ("This doesn't affect path components other than the last", `follow_symlinks.rs`); links in the middle are followed while they stay inside the folder. On Windows only opening is relative to a handle; rename, remove, listing and making links go by full path (`get_path.rs`: `file_path.join(path)`; `rename_unchecked.rs`: `fs::rename(old_full_path, new_full_path)`). Wasmtime, its main user, stopped depending on it in 2026 (pull request 14219, "Vendor the cap-primitives crate"), and it has one main maintainer.
- **C. `rustix` on macOS and Linux, and a small Windows module of our own.** `rustix` gives safe functions for `openat`, `mkdirat`, `renameat`, `unlinkat`, `statat`, `fchmod`, `symlinkat` and folder listing from a handle; it is already in `Cargo.lock` through a test dependency. Windows needs about 250 to 350 lines with 10 to 15 small `unsafe` blocks: `NtCreateFile` with a root folder, `FILE_OPEN_REPARSE_POINT` and `OBJ_DONT_REPARSE` (as the standard library's own `remove_dir_all` does), `SetFileInformationByHandle` to rename and delete, and `GetFileInformationByHandleEx` to list.

## Decision

**C.**

- **One module, in the platform crate**: a folder handle type with the operations Bolt needs (open a folder or file inside it, list it, read an entry's details, make, rename over, remove, make a link, read a link, set the executable bit), each refusing a link or other reparse point at the name it is given. Paths are walked one name at a time from a handle, never joined into a path a call looks up.
- **macOS and Linux** use `rustix`, a new direct dependency (licence Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT), with no new `unsafe`.
- **Windows** uses its own calls in the platform crate. The owner reviews each `unsafe` line before milestone 3 is merged, as decision 022 requires.
- **Everything** that walks or writes the project folder goes through it: `save`, `status`, `goto`, `discard`, and the fixed folders inside `.bolt`.

## Consequences

- Security.md T3, T19 and T20 lose their known limits: a folder swapped for a link mid-walk is refused, not followed.
- Making a link on Windows has no handle-relative call, so it is made by path while Bolt holds the folder open.
- To test on a real machine: OneDrive files opened this way, renaming a folder while Bolt holds one inside it, and paths over 260 characters.
- A walk that holds every folder on its way open needs one handle per level, at most 4,096 (the depth limit); folders are closed as the walk leaves them.
