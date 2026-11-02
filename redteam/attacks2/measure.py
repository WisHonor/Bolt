#!/usr/bin/env python3
"""Runs one command, no memory cap, and prints its exit status, time and peak memory.
Usage: python3 -I measure.py <timeout-secs> <cwd> <cmd...>"""
import resource, subprocess, sys, time

limit, cwd, cmd = float(sys.argv[1]), sys.argv[2], sys.argv[3:]
start = time.time()
try:
    p = subprocess.run(cmd, cwd=cwd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                       timeout=limit)
    status = p.returncode if p.returncode >= 0 else f"killed by signal {-p.returncode}"
    err = p.stderr.decode(errors="replace")[:300].strip()
except subprocess.TimeoutExpired:
    status, err = f"still running after {limit:.0f} s (killed)", ""
peak_mb = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss / 1024
print(f"{' '.join(cmd[-2:])}: exit {status}, {time.time() - start:.1f} s, peak {peak_mb:.0f} MB")
if err:
    print(f"  stderr: {err}")
