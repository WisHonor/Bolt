"""Runs a command and prints its exit code, time and peak memory (KiB). Sandbox only."""
import resource
import subprocess
import sys
import time

start = time.monotonic()
done = subprocess.run(sys.argv[1:], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
took = time.monotonic() - start
peak = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
print(f"code {done.returncode} in {took:.2f}s, peak {peak} KiB; {done.stderr[:200]!r}")
