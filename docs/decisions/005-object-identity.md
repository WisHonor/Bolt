# 005 — Object identity: content fingerprints

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 5

## Context

Every stored object is identified by a fingerprint (hash) of its contents. Bolt uses fingerprints to find data, notice changes, store duplicates once and detect corruption or tampering. These rules are the bottom brick of the format: changing them later means converting every repository. The hardest question is big files, which are stored as chunks: is a file identified by its contents or by the chunk list it was cut into?

## Options

- **By contents.** A file's ID is the fingerprint of its whole contents; chunking is a storage detail. Same bytes always give the same ID, so presets and chunking settings can change freely. A download is fully confirmed only once complete, though each chunk is still checked on its own. (Git, Git LFS.)
- **By chunk list.** A file's ID is the fingerprint of its list of chunk fingerprints (a Merkle tree). Every piece is verifiable on arrival, but the ID depends on chunking settings, which would have to be frozen forever. (Hugging Face Xet, IPFS.)

## Decision

**Files are identified by their contents.**
- **Algorithm:** BLAKE3 with 256-bit IDs, displayed as lowercase hex; the UI shows the shortest unambiguous prefix. The algorithm is declared once per repository and never mixed within one; changing it later means a one-time conversion.
- **Flavors:** file contents and chunks are fingerprinted with plain BLAKE3, so a file's ID equals what `b3sum` prints. Every structured object type (folder listings, saves, chunk lists, and any later type) uses BLAKE3's key-derivation mode with its own fixed context string, so objects of different types can never share an ID or be mistaken for one another.
- **Chunk lists:** a big file's chunk list is a separate object linked to the file's ID; a reassembled file is verified against that ID before use.
- **Folder listings:** one per folder, like Git. Each entry holds name, entry kind (decision 004), ID and size, sorted by name in byte order.
- **Encoding:** a small hand-written binary format, specified in the storage spec with test vectors. Each object has exactly one valid encoding; readers reject anything the writer wouldn't produce (unsorted or duplicate entries, trailing bytes, non-minimal numbers). Serde-based formats are allowed only for data that is never fingerprinted, such as caches and indexes.
- **Known but not downloaded:** an object can be referenced without its contents being present. Every command treats this as a normal state, and `bolt verify` distinguishes "not downloaded" from "missing because something broke".

## Consequences

- Presets and chunking parameters can change at any time without changing any ID, which keeps the "change anytime" promise of the `bolt start` screen.
- Chunking parameters can be tuned after measuring real assets (topic 9).
- Verified streaming of partial downloads (Bao, built on BLAKE3's tree structure) is a later optimization, not a format change.
- The storage spec must define every object's byte layout, the context strings and test vectors before implementation (topic 18).
- Sizes in folder listings let status, listings and download planning work without file contents.
- A giant single folder produces a large listing that is rewritten whenever any entry changes; splitting such listings can come later through the format version.
