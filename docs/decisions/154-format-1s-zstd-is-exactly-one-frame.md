# 154 — Format 1's zstd is exactly one frame

- **Status:** Accepted
- **Date:** 2026-11-12
- **Agenda topic:** none (the v0.1 release: the final builds)
- **Decided by:** the owner
- **Supersedes in part:** [decision 148](148-what-format-1s-spec-says-before-it-freezes.md) (its bullet on §11, zstd as v0.1 reads it, and nothing else)

## Context

zstd is the compression Opalro stores with ([decision 009](009-storage-engine.md)). A frame is one compressed piece. Decision 148 wrote down how v0.1 read compression 1: one or more frames to the end of the stored bytes, with skippable frames passed over. The formats from before zstd 1.0 (v0.4 to v0.7) were read too, because the zstd library was built with them. Opalro itself writes one frame.

The last read of the format before the freeze (finding F1, the one it called blocking) said that a reader frozen this loosely binds every later Opalro, and any program written from the spec, to read bytes no Opalro writes. That includes the pre-1.0 formats, which only one C library still reads. Tightening the reader is a few lines before the freeze and impossible after it.

## Options

- **A. Keep decision 148's reading.** No code changes. Format 1 freezes reading several frames, skippable frames and pre-1.0 frames.
- **B. Exactly one frame.** A reader refuses anything else.

## Decision

**B.** In format 1, compression 1 is exactly one zstd frame (RFC 8878, magic `28 B5 2F FD`):

- no skippable frame, before or after it;
- no dictionary;
- a window of at most 2^27 bytes (128 MiB; a single-segment frame's window is its content size);
- a content checksum is optional, and is checked when present;
- its content size, if it gives one, equal to the entry's uncompressed size;
- nothing after it.

A reader refuses anything else as damage. Opalro is built without zstd's legacy decoders, so no path reads a frame from before zstd 1.0.

This replaces decision 148's zstd bullet and nothing else. The review's other findings stay where decision 148 left them.

## Consequences

- Every frame Opalro writes is such a frame: level 3, no dictionary, and a window far under 2^27 bytes. It gives a content size when it compresses an object in one go, and none when it streams a file it stores whole. So no project v0.1 writes changes meaning. A pack made by hand or by another program, holding several frames, a skippable frame or a pre-1.0 frame, is now damaged, and `opalro verify` says so.
- Small entries read in one call and larger ones read as a stream refuse the same bytes, with the same words.
- A program written from [storage-format.md](../storage-format.md) §11 needs only RFC 8878 and these rules.
- The rulings that pin the details are in [the final builds' rulings](../plans/2026-11-12-v0.1-final-rulings.md): no `Dictionary_ID` field at all; the header's two spare bits clear; each block's size checked from its header on both read paths, since zstd's one-call decoder doesn't check a raw or RLE block; and the one case a header can't show, a compressed block that unpacks past the limit, which a reader may read.
