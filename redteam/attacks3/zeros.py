"""A one-commit repository whose only file is <MiB> MiB of zeros, stored as one loose object that
Python's zlib compressed at <level> (its header and data in one stream, as Git writes it).
Usage: python3 -I zeros.py <dir> <KiB> <level> [chunk-KiB]"""
import os
import sys
import zlib

sys.path.insert(0, "/work/redteam/attacks3")
from gitrepo import Repo, oid, commit_bytes

root, kib, level = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
step = (int(sys.argv[4]) if len(sys.argv) > 4 else 1024) << 10
size = kib << 10
repo = Repo(root)
import hashlib
h = hashlib.sha1(b"blob %d\0" % size)
z = zlib.compressobj(level)
parts = [z.compress(b"blob %d\0" % size)]
left = size
while left:
    n = min(step, left)
    block = bytes(n)
    h.update(block)
    parts.append(z.compress(block))
    left -= n
parts.append(z.flush())
blob = h.digest()
repo.file("objects/%s/%s" % (blob.hex()[:2], blob.hex()[2:]), b"".join(parts))
tree = repo.loose(b"tree", b"100644 zeros\0" + blob)
repo.ref("refs/heads/main", repo.loose(b"commit", commit_bytes(tree, [], b"zeros\n")))
