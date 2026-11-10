#!/usr/bin/env bash
# Milestone 6 speed red team, session RT13 (decision 024). Never run on the host.
#   redteam/attacks12/run.sh proofs           the proof tests, redteam13.rs
#   redteam/attacks12/run.sh explore <name>   one exploration file of this folder (rt13_*.rs),
#                                             copied into crates/bolt-vcs/tests/ for the run
#   redteam/attacks12/run.sh script <file>    one of the shell or Python probes of this folder
# Scripts compare /work/target/release/bolt with main's /work/_main/target/release/bolt: build
# both first (cargo build -j 2 --release -p bolt-vcs --features fault-injection at /work, and
# cargo build -j 2 --release -p bolt-vcs at /work/_main).
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
SANDBOX="$HERE/../sandbox.sh"
case "${1:-}" in
proofs)
    bash "$SANDBOX" run sh -c 'cargo test -j 2 -p bolt-vcs --features fault-injection \
        --test redteam13 -- --ignored --test-threads=2'
    ;;
explore)
    name=${2:?usage: run.sh explore <rt13_name>}
    bash "$SANDBOX" run sh -c "cp redteam/attacks12/$name.rs crates/bolt-vcs/tests/ && \
        timeout 2400 cargo test -j 2 -p bolt-vcs --features fault-injection --test $name -- \
        --test-threads=1 --nocapture; rm crates/bolt-vcs/tests/$name.rs"
    ;;
script)
    file=${2:?usage: run.sh script <file>}
    case $file in
    *.py) bash "$SANDBOX" run timeout 3000 python3 -I "redteam/attacks12/$file" "${@:3}" ;;
    *) bash "$SANDBOX" run timeout 3000 bash "redteam/attacks12/$file" "${@:3}" ;;
    esac
    ;;
*)
    sed -n '2,9p' "$0" >&2
    exit 2
    ;;
esac
