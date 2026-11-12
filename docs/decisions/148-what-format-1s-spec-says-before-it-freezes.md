# 148 — What format 1's spec says before it freezes

- **Status:** Accepted
- **Date:** 2026-11-11
- **Agenda topic:** none (the v0.1 release: the package)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 121](121-format-1-freezes-at-v0.1.md) (what the freeze covers)
- **Superseded in part by:** [decision 150](150-the-name-is-opalro.md) (the names: Bolt is now Opalro, and `bolt` is now `opal`)

## Context

**In plain words.** Format 1 freezes when v0.1 is published (decision 121): from then on, any Bolt must read what v0.1 wrote, and a second program written from [storage-format.md](../storage-format.md) alone must read the same repositories the same way. So before the freeze, the spec was read once more against the code. In most places the two agreed. In some, the code had rules the spec didn't state. In two, the spec couldn't keep its word as it stood:

1. **The conflict record (§8).** Decision 121 freezes "how every object is encoded and fingerprinted (§1 to §9)", but §8 says the conflict record is a draft "finalised before v0.2". It can't be both. v0.1 has no code that reads the record, and it refuses a repository that lists `feature conflicts`.
2. **The ignore rules (§12).** They followed Git "as Git 2.52 and later have it". If a later Git changes a quirk, two honest readers of format 1 could follow different Gits. No stored byte depends on these rules: they only decide what a later save picks up.

The ruling for this read was: say in the spec exactly what v0.1's code already does, with no code change, no vector change and no byte on disk changed.

## Options

**The conflict record**
- **A. Freeze its layout now,** as drafted. v0.2's conflict work would then be bound to a layout no code has ever written or read.
- **B. Freeze only the reservation,** that is, the numbers and the context string, plus what v0.1 does when it meets them. The layout arrives with the `conflicts` feature, which v0.1 already refuses.

**The ignore rules**
- **A. Pin them** to Git's behaviour at the versions the spec names (2.52, and 2.56 for `:(optional)`).
- **B. Follow Git as it changes,** since no stored byte depends on them. But then the frozen spec would point at a moving target.

## Decision

1. **The conflict record: B.** §8's layout does not freeze with format 1. It arrives with `feature conflicts`. What freezes is the reservation: listing entry kind 6, pack object kind 19, and the context string `bolt-vcs 2026-10-31 conflict v1`. Also frozen is what v0.1 does on meeting them in a repository without the feature. It refuses `feature conflicts` as newer. It decodes a listing holding kind 6, takes a kind-19 object and checks its fingerprint without decoding it, and write-out leaves a kind-6 entry unwritten. `tidy` stops, and `verify` checks the object by its fingerprint only and reports a needed listing holding kind 6 as damaged.
2. **The ignore rules: A.** The Git versions the spec names pin the rules for format 1.
3. **The rest, written down as the code already does it** (storage-format.md):
   - §3: the exact `.bolt` spelling rule. Take out the code points macOS ignores, cut at the first `:` or `\`, drop trailing dots and spaces, fold ASCII case only, and check against `.bolt` and `bolt~1` to `bolt~4`.
   - §3: listing sizes. Kinds 4 to 6 must be 0. For kinds 1 to 3, the size must be what the ID names; a listing where it isn't is damaged, `verify` reports it, and write-out refuses a file or executable file of the wrong length.
   - §4: when the exact Git bytes promise holds. It holds given the Git IDs of the commit's own tree and parents. Import counts the commits whose trees can't be written back the same way. Seconds are rounded down, and the hours have at least two digits.
   - §9: a line name's letters and digits are Rust's `char::is_alphanumeric`. That is Unicode's Alphabetic property, or general category Nd, Nl or No.
   - §11: zstd as v0.1 reads it. One or more frames, skippable frames passed over, and the pre-1.0 formats v0.4 to v0.7 read. Refused: the formats v0.1 to v0.3, dictionaries, a wrong content checksum, and windows over 2^27 bytes. Also in §11: compression 0 needs equal sizes (chunk lists excepted); every `*.pack` file is read; and gaps in the entry area are allowed, with overlap refused only in version 1.
   - §12: `bolt.toml` may start with a UTF-8 byte order mark. Nesting deeper than 80, or a key of more than 80 dotted parts, makes the file "not TOML".

## Consequences

- The spec now says what v0.1 reads and refuses, so a second reader written from it can agree with v0.1 on every repository. Some of what v0.1 reads is broader than Bolt ever writes (several zstd frames, the older zstd formats); writing it down keeps it readable after the freeze.
- v0.2 defines the conflict record's layout under `feature conflicts` without breaking format 1.
- A later Git that changes an ignore quirk changes nothing for format 1. Following it would take a new decision, and no stored byte would change.
- The review's other findings are left to the owner: refusal vectors, smaller vector gaps, the extra-field limit, room to grow in records, and how `bolt.toml` rules match files.
