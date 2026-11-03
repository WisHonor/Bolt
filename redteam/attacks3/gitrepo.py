"""Tiny Git repository writer for the milestone 2 red-team scripts (no git in the sandbox).

Writes loose objects, packs with v2 indexes (whole objects and OFS_DELTAs), refs and HEAD,
byte-exact as Git would. Run inside the sandbox only.
"""
import hashlib
import os
import struct
import zlib


def oid(kind, data):
    return hashlib.sha1(b"%s %d\0" % (kind, len(data)) + data).digest()


class Repo:
    def __init__(self, root):
        self.root = root
        self.git = os.path.join(root, ".git")
        for d in ("objects/pack", "refs/heads", "refs/tags"):
            os.makedirs(os.path.join(self.git, d), exist_ok=True)
        self.file("config", b"[core]\n\trepositoryformatversion = 0\n")
        self.file("HEAD", b"ref: refs/heads/main\n")

    def file(self, rel, data):
        path = os.path.join(self.git, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(data)

    def loose(self, kind, data):
        i = oid(kind, data)
        h = i.hex()
        self.file("objects/%s/%s" % (h[:2], h[2:]),
                  zlib.compress(b"%s %d\0" % (kind, len(data)) + data))
        return i

    def ref(self, name, i):
        self.file(name, i.hex().encode() + b"\n")


def varint(n):
    out = bytearray()
    while n >= 0x80:
        out.append((n & 0x7F) | 0x80)
        n >>= 7
    out.append(n)
    return bytes(out)


def entry_header(code, size):
    out = bytearray()
    byte = (code << 4) | (size & 0x0F)
    size >>= 4
    while size:
        out.append(byte | 0x80)
        byte = size & 0x7F
        size >>= 7
    out.append(byte)
    return bytes(out)


def ofs(distance):
    out = [distance & 0x7F]
    distance >>= 7
    while distance:
        distance -= 1
        out.append(0x80 | (distance & 0x7F))
        distance >>= 7
    return bytes(reversed(out))


def copy_op(offset, length):
    op, args = 0x80, bytearray()
    for i, bit in enumerate((0x01, 0x02, 0x04, 0x08)):
        b = (offset >> (8 * i)) & 0xFF
        if b:
            op |= bit
            args.append(b)
    for i, bit in enumerate((0x10, 0x20, 0x40)):
        b = (length >> (8 * i)) & 0xFF
        if b:
            op |= bit
            args.append(b)
    return bytes([op]) + bytes(args)


def insert_ops(data):
    out = bytearray()
    for i in range(0, len(data), 127):
        chunk = data[i:i + 127]
        out.append(len(chunk))
        out += chunk
    return bytes(out)


TYPES = {b"commit": 1, b"tree": 2, b"blob": 3, b"tag": 4}


class PackWriter:
    """Streams a pack to disk; entries are ("whole", kind, data, id) or ("ofs", base_index,
    delta_bytes, id), where id is the object's real ID (the caller computes it)."""

    def __init__(self, repo, count, level=6):
        self.repo = repo
        self.tmp = os.path.join(repo.git, "objects/pack/tmp.pack")
        self.f = open(self.tmp, "wb")
        self.sha = hashlib.sha1()
        self.pos = 0
        self.rows = []  # (id, crc, offset)
        self.level = level
        self._w(b"PACK" + struct.pack(">II", 2, count))

    def _w(self, b):
        self.f.write(b)
        self.sha.update(b)
        self.pos += len(b)

    def whole(self, kind, data, i, compressed=None):
        at = self.pos
        body = entry_header(TYPES[kind], len(data)) + (compressed or zlib.compress(data, self.level))
        self.rows.append((i, zlib.crc32(body), at))
        self._w(body)
        return at

    def whole_sized(self, kind, size, compressed, i):
        at = self.pos
        body = entry_header(TYPES[kind], size) + compressed
        self.rows.append((i, zlib.crc32(body), at))
        self._w(body)
        return at

    def ofs_delta(self, base_at, delta, i):
        at = self.pos
        body = entry_header(6, len(delta)) + ofs(at - base_at) + zlib.compress(delta, self.level)
        self.rows.append((i, zlib.crc32(body), at))
        self._w(body)
        return at

    def finish(self):
        checksum = self.sha.digest()
        self.f.write(checksum)
        self.f.close()
        rows = sorted(self.rows)
        idx = bytearray(b"\xfftOc" + struct.pack(">I", 2))
        counts = [0] * 256
        for r in rows:
            counts[r[0][0]] += 1
        total = 0
        for c in counts:
            total += c
            idx += struct.pack(">I", total)
        for r in rows:
            idx += r[0]
        for r in rows:
            idx += struct.pack(">I", r[1])
        large = []
        for r in rows:
            if r[2] >= 0x80000000:
                idx += struct.pack(">I", 0x80000000 | len(large))
                large.append(r[2])
            else:
                idx += struct.pack(">I", r[2])
        for at in large:
            idx += struct.pack(">Q", at)
        idx += checksum
        idx += hashlib.sha1(bytes(idx)).digest()
        name = "pack-" + checksum.hex()
        os.rename(self.tmp, os.path.join(self.repo.git, "objects/pack", name + ".pack"))
        self.repo.file("objects/pack/%s.idx" % name, bytes(idx))
        return name


EMPTY_TREE = b""
PEOPLE = b"author A <a@example.com> 1700000000 +0000\ncommitter A <a@example.com> 1700000000 +0000\n"


def commit_bytes(tree, parents, message):
    out = b"tree " + tree.hex().encode() + b"\n"
    for p in parents:
        out += b"parent " + p.hex().encode() + b"\n"
    return out + PEOPLE + b"\n" + message
