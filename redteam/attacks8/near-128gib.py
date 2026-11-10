#!/usr/bin/env python3
"""Milestone 5 red team, session 2: files at the cut-off between chunked and whole, 2^37 bytes
(128 GiB, the last size cut into chunks) and 2^37 + 1 (stored whole, read twice). Sparse files
of zeros in /tmp, so they take no disk; nothing is written out. For each: `bolt save`, then
`bolt status` after its time changes, then `bolt status --full`, each with its time and peak
resident memory. Sandbox only:
    bash redteam/sandbox.sh run python3 -I redteam/attacks8/near-128gib.py
"""
import os, shutil, subprocess, sys, tempfile, time

BOLT = os.environ.get("BOLT", "/work/target/release/bolt")
SIZES = [int(a) for a in sys.argv[1:]] or [1 << 37, (1 << 37) + 1]


def run(args, cwd, env):
    start = time.time()
    p = subprocess.Popen([BOLT, *args], cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    peak = 0
    while p.poll() is None:
        try:
            with open("/proc/%d/status" % p.pid) as f:
                for line in f:
                    if line.startswith("VmHWM:"):
                        peak = max(peak, int(line.split()[1]))
        except OSError:
            pass
        time.sleep(0.2)
    out, err = p.communicate()
    return p.returncode, time.time() - start, peak // 1024, (out + err).decode(errors="replace")


def main():
    env = dict(os.environ, BOLT_CONFIG_DIR=tempfile.mkdtemp(), BOLT_USER_NAME="T",
               BOLT_USER_EMAIL="t@example.com")
    for size in SIZES:
        d = tempfile.mkdtemp(dir="/tmp")
        try:
            subprocess.run([BOLT, "start"], cwd=d, env=env, capture_output=True, check=True)
            f = os.path.join(d, "zeros.bin")
            with open(f, "wb") as h:
                h.truncate(size)
            for args in (["save", "--yes", "big"], ["status"], ["status", "--full"]):
                if args == ["status"]:
                    os.utime(f, (time.time() + 5, time.time() + 5))
                code, secs, peak, said = run(args, d, env)
                print("%d bytes: bolt %-22s exit %s in %6.1f s, peak %4d MiB: %s" % (
                    size, " ".join(args), code, secs, peak, " ".join(said.split())[:160]), flush=True)
            used = subprocess.run(["du", "-sh", os.path.join(d, ".bolt")], capture_output=True, text=True).stdout.split()[0]
            print("%d bytes: .bolt holds %s" % (size, used), flush=True)
        finally:
            shutil.rmtree(d, ignore_errors=True)


main()
