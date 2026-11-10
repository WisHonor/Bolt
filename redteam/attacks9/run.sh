#!/usr/bin/env bash
# Milestone 6 red team, presets and polish (decision 024). Never run on the host.
#   redteam/attacks9/run.sh proofs           the proof tests, redteam10.rs
#   redteam/attacks9/run.sh explore <name>   one exploration file of this folder (rt10_*.rs),
#                                            copied into crates/bolt-vcs/tests/ for the run
#   redteam/attacks9/run.sh script <file>    one of the shell probes of this folder
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
SANDBOX="$HERE/../sandbox.sh"
case "${1:-}" in
proofs)
    bash "$SANDBOX" run sh -c 'cargo test -j 2 -p bolt-vcs --all-features \
        --test redteam10 -- --ignored --test-threads=2'
    ;;
explore)
    name=${2:?usage: run.sh explore <rt10_name>}
    bash "$SANDBOX" run sh -c "cp redteam/attacks9/$name.rs crates/bolt-vcs/tests/ && \
        cargo test -j 2 -p bolt-vcs --all-features --test $name -- \
        --test-threads=1 --nocapture; rm crates/bolt-vcs/tests/$name.rs"
    ;;
script)
    file=${2:?usage: run.sh script <file>}
    # The scripts run the debug build with fault injection.
    bash "$SANDBOX" run sh -c "cargo build -j 2 -p bolt-vcs --features fault-injection -q && \
        bash redteam/attacks9/$file ${*:3}"
    ;;
*)
    sed -n '2,6p' "$0" >&2
    exit 2
    ;;
esac
