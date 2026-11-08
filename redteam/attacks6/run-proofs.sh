#!/usr/bin/env bash
# Runs the second milestone 4 session's proof tests in the red-team sandbox (decision 024).
# Never run them on the host.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
bash "$HERE/../sandbox.sh" run cargo test -j 2 -p bolt-vcs --features fault-injection \
    --test redteam7 -- --ignored --test-threads=2
