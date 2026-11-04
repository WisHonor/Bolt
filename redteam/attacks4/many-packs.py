"""Builds a one-commit repository of N small files (one pack), beside P empty packs: each a valid
pack of no objects and its index, the same 1,104 bytes every time. Every object lookup looks in
every pack first.
Usage: python3 -I many-packs.py <dir> <N> <P>"""
import hashlib
import os
import struct
import sys

sys.path.insert(0, "/work/redteam/attacks3")
from gitrepo import Repo, PackWriter, oid, commit_bytes

root, n, p = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
repo = Repo(root)
pack = PackWriter(repo, n + 2)
entries = []
for i in range(n):
    data = b"file %d\n" % i
    i_ = oid(b"blob", data)
    pack.whole(b"blob", data, i_)
    entries.append((b"f%06d" % i, i_))
tree_data = b"".join(b"100644 " + name + b"\0" + i for name, i in entries)
tree = oid(b"tree", tree_data)
pack.whole(b"tree", tree_data, tree)
commit = commit_bytes(tree, [], b"Many files\n")
tip = oid(b"commit", commit)
pack.whole(b"commit", commit, tip)
pack.finish()
repo.ref("refs/heads/main", tip)
# An empty pack and its index: header, fan-out of zeros, the pack's checksum, the index's own.
empty = b"PACK" + struct.pack(">II", 2, 0)
empty += hashlib.sha1(empty).digest()
idx = b"\xfftOc" + struct.pack(">I", 2) + bytes(1024) + empty[-20:]
idx += hashlib.sha1(idx).digest()
folder = os.path.join(repo.git, "objects/pack")
for k in range(p):
    # Named to sort before the real pack, so every lookup passes them all first.
    name = os.path.join(folder, "pack-0%039x" % k)
    with open(name + ".pack", "wb") as f:
        f.write(empty)
    with open(name + ".idx", "wb") as f:
        f.write(idx)
