# 098 — A blob is also judged as Git LFS would read it with CRLF line ends

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 5's red-team fixes)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 097](097-red-team-m5-session-2-rulings.md) (which blobs that don't come in as Git LFS files are named in `lfs_lookalikes`)

## Context

On checkout, Git can turn each line end into CR LF before Git LFS's smudge filter reads a file's first 1,024 bytes. It does this under `eol=crlf`, or under `core.autocrlf=true`, which Git for Windows' installer sets. So what Git LFS reads isn't always the blob's own bytes (RT9-2).

Decision 097 answered with a wider warning: a blob over 1,024 bytes whose first 1,024 bytes *look like* a pointer, by decision 037's looser test, is named in `lfs_lookalikes`. Its reason was that a conversion only pushes bytes out, so such a blob still looks like a pointer.

The review of that fix found shapes the reason misses. Git LFS's parser trims leading whitespace and accepts its first, alpha format (`version http://git-media.io/v/2`). Decision 037's test does neither. So a pointer led by a newline, or written in the alpha format, with junk that the conversion pushes out of the 1,024 bytes, still came in silently as text, while Git LFS users got the pointer's file. A file of 1,024 bytes or less can do the same once the conversion grows it. Bolt's own parser, `lfs_pointer`, already models both the trimming and the alpha format.

## Options

- **A. Run Git LFS's parser on the CRLF form too.** Take the blob's first 1,024 bytes as Git would hand them over with every LF (one not already after a CR) turned into CR LF. Cut them at 1,024 bytes, and run `lfs_pointer` on them, for every blob that doesn't come in as a Git LFS file, of any size. A conversion only adds bytes, so the first 1,024 converted bytes come from at most the blob's first 1,024. Nothing more is read.
- **B. Loosen decision 037's test** to trim whitespace and accept the alpha format. That copies Git LFS's parser by hand in a second place, against one place per rule, and still misses whatever else the parser accepts.
- **C. Call it a known limit.**

## Decision

**A.**

A blob that doesn't come in as a Git LFS file comes in as its own bytes. It is named in `lfs_lookalikes` when any of these holds:
- Git LFS's parser accepts its first 1,024 bytes (decision 094);
- Git LFS's parser accepts the first 1,024 bytes of its CRLF form (this decision);
- its first 1,024 bytes look like a pointer (decision 097).

The CRLF form is the one conversion modelled. It is the one Git for Windows turns on by default, and it changes only line ends.

A warning where Git wouldn't in fact have converted the file (a binary file, or `-text`) costs nothing but the warning. `ident` and working-tree encodings stay a known limit, beside the deferred row on judging Git LFS files by `.gitattributes`. So does the other way round (a pointer the conversion pushes past 1,024 bytes).

## Consequences

- The check lives in git/object.rs, with Git LFS's other pointer rules. Unit tests cover the review's three shapes: a newline before the pointer, a short file led by a newline, and the alpha format.
- The warning and its docs say that Git LFS *may* check out a look-alike as its pointer's file, whatever its size.
