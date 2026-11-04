"""Tag repositories for the tag-chain and tag-list measurements. One commit on main, then:
  chain <refs>: a chain of 16 small tag objects, and <refs> tag refs naming its outermost one;
  fat <refs>: one tag object of almost 1 MiB (a compressible message), named by <refs> tag refs.
Usage: python3 -I tags.py <dir> chain|fat <refs>"""
import sys

sys.path.insert(0, "/work/redteam/attacks3")
from gitrepo import Repo, oid, commit_bytes

root, kind, refs = sys.argv[1], sys.argv[2], int(sys.argv[3])
repo = Repo(root)
tree = repo.loose(b"tree", b"")
commit = repo.loose(b"commit", commit_bytes(tree, [], b"one\n"))
repo.ref("refs/heads/main", commit)


def tag(target, kind, message):
    head = b"object %s\ntype %s\ntag t\ntagger T <t@example.com> 1700000000 +0000\n\n" % (
        target.hex().encode(), kind)
    return repo.loose(b"tag", head + message)


if kind == "chain":
    top = tag(commit, b"commit", b"v1\n")
    for _ in range(15):
        top = tag(top, b"tag", b"again\n")
else:
    top = tag(commit, b"commit", b"x" * ((1 << 20) - 200))
lines = [b"# pack-refs with: peeled fully-peeled sorted "]
lines += [b"%s refs/tags/t%07d" % (top.hex().encode(), i) for i in range(refs)]
repo.file("packed-refs", b"\n".join(lines) + b"\n")
