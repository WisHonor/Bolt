"""One commit whose only file is a <MiB> MiB blob stored as a delta on another blob of that size,
itself a delta on 1 MiB of zeros: how much memory streaming a deltified blob takes.
Usage: python3 -I big-blob.py <dir> <MiB>"""
import hashlib
import struct
import sys

sys.path.insert(0, "/work/redteam/attacks3")
from gitrepo import Repo, PackWriter, oid, commit_bytes, copy_op, insert_ops, varint

root, mib = sys.argv[1], int(sys.argv[2])
size = mib << 20
repo = Repo(root)
pack = PackWriter(repo, 5)
fake = lambda k: bytes([0xFE]) + struct.pack(">Q", k) + bytes(11)
at = pack.whole(b"blob", bytes(1 << 20), fake(0))
at = pack.ofs_delta(at, varint(1 << 20) + varint(size) + copy_op(0, 1 << 20) * mib, fake(1))
# The file: the same size, its last 8 bytes its own.
delta = varint(size) + varint(size)
done = 0
while done < size - 8:
    step = min(0xFF0000, size - 8 - done)
    delta += copy_op(done, step)
    done += step
tail = b"the end!"
delta += insert_ops(tail)
h = hashlib.sha1(b"blob %d\0" % size)
zeros = bytes(1 << 20)
for _ in range(mib - 1):
    h.update(zeros)
h.update(bytes((1 << 20) - 8) + tail)
blob = h.digest()
pack.ofs_delta(at, delta, blob)
tree_data = b"100644 big.bin\0" + blob
tree = oid(b"tree", tree_data)
pack.whole(b"tree", tree_data, tree)
data = commit_bytes(tree, [], b"big file\n")
commit = oid(b"commit", data)
pack.whole(b"commit", data, commit)
pack.finish()
repo.ref("refs/heads/main", commit)
