#!/usr/bin/env bash
# Milestone 5 red team (decision 024). Never run on the host.
#   redteam/attacks7/run.sh proofs           the proof tests, redteam8.rs
#   redteam/attacks7/run.sh explore <name>   one exploration file of this folder (rt8_*.rs),
#                                            copied into crates/bolt-vcs/tests/ for the run
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
SANDBOX="$HERE/../sandbox.sh"
case "${1:-}" in
proofs)
    bash "$SANDBOX" run sh -c 'export CARGO_TARGET_DIR=/tmp/target; cargo test -j 2 -p bolt-vcs \
        --features fault-injection --test redteam8 -- --ignored --test-threads=2'
    ;;
explore)
    name=${2:?usage: run.sh explore <rt8_name>}
    bash "$SANDBOX" run sh -c "cp redteam/attacks7/$name.rs crates/bolt-vcs/tests/ && \
        export CARGO_TARGET_DIR=/tmp/target; cargo test -j 2 -p bolt-vcs --features \
        fault-injection --test $name -- --test-threads=1 --nocapture; \
        rm crates/bolt-vcs/tests/$name.rs"
    ;;
*)
    sed -n '2,5p' "$0" >&2
    exit 2
    ;;
esac
