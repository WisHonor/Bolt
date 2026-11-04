"""Builds a chain of N commits that all carry one random 1 MB message: Git stores the first whole
and each later one as a ~80-byte delta on it, so the repository is about 1 MB. Bolt stores each
save whole, message included.
Usage: python3 -I messages.py <dir> <N> [message bytes]"""
import random
import string
import sys

sys.path.insert(0, "/work/redteam/attacks3")
from gitrepo import Repo, PackWriter, oid, commit_bytes, copy_op, insert_ops, varint

root, n = sys.argv[1], int(sys.argv[2])
size = int(sys.argv[3]) if len(sys.argv) > 3 else 1_000_000
rng = random.Random(4)
message = "".join(rng.choices(string.ascii_letters + string.digits, k=size - 1)).encode() + b"\n"
repo = Repo(root)
pack = PackWriter(repo, 2 + n)
tree = oid(b"tree", b"")
pack.whole(b"tree", b"", tree)
base = commit_bytes(tree, [], message)
tip = oid(b"commit", base)
base_at = pack.whole(b"commit", base, tip)
head = 46  # "tree <hex>\n"
for _ in range(n):
    data = base[:head] + b"parent " + tip.hex().encode() + b"\n" + base[head:]
    delta = (varint(len(base)) + varint(len(data)) + copy_op(0, head)
             + insert_ops(data[head:head + 48]) + copy_op(head, len(base) - head))
    tip = oid(b"commit", data)
    pack.ofs_delta(base_at, delta, tip)
pack.finish()
repo.ref("refs/heads/main", tip)
