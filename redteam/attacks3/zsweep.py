"""Writes zlib streams of assorted contents and sizes (as Git writes objects: level 6 by default,
header first) into <dir>, each named <inflated-size>_<what>.z, for the inflate sweep.
Usage: python3 -I zsweep.py <dir>"""
import os
import random
import sys
import zlib

out = sys.argv[1]
os.makedirs(out, exist_ok=True)
rng = random.Random(1)


def write(what, data, level=6):
    raw = b"blob %d\0" % len(data) + data
    with open(os.path.join(out, "%d_%s.z" % (len(raw), what)), "wb") as f:
        f.write(zlib.compress(raw, level))


text = b"".join(b"line %d of a log file, nothing much to see here\n" % i for i in range(200000))
for mib in (1, 4, 16, 48):
    n = mib << 20
    write("zeros-%dMiB" % mib, bytes(n))
    write("text-%dMiB" % mib, (text * (n // len(text) + 1))[:n])
    # A sparse file: 4 KiB pages, one in eight with random bytes, the rest zeros.
    pages = bytearray(n)
    for p in range(0, n, 4096 * 8):
        pages[p:p + 4096] = rng.randbytes(4096)
    write("sparse-%dMiB" % mib, bytes(pages))
    write("onebyte-%dMiB" % mib, b"A" * n, 9)
for kib in (100, 300, 700, 1500, 3000, 6000, 12000, 24000, 50000, 100000):
    write("zeros-%dKiB" % kib, bytes(kib << 10), 6)
    write("zeros9-%dKiB" % kib, bytes(kib << 10), 9)
