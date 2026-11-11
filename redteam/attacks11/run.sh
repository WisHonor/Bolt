#!/usr/bin/env bash
# Milestone 6 speed red team, session RT12 (decision 024). Never run on the host.
#   redteam/attacks11/run.sh proofs           the proof tests, redteam12.rs
#   redteam/attacks11/run.sh explore <name>   one exploration file of this folder (rt12_*.rs),
#                                             copied into crates/bolt-vcs/tests/ for the run
#   redteam/attacks11/run.sh script <file>    one of the shell or Python probes of this folder
# Scripts compare /work/target/release/bolt with main's /work/_main/target/release/bolt: build
# both first (cargo build -j 2 --release -p bolt-vcs, at /work and at /work/_main).
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
SANDBOX="$HERE/../sandbox.sh"
case "${1:-}" in
proofs)
    bash "$SANDBOX" run sh -c 'cargo test -j 2 -p bolt-vcs --features fault-injection \
        --test redteam12 -- --ignored --test-threads=2'
    ;;
explore)
    name=${2:?usage: run.sh explore <rt12_name>}
    bash "$SANDBOX" run sh -c "cp redteam/attacks11/$name.rs crates/bolt-vcs/tests/ && \
        cargo test -j 2 -p bolt-vcs --features fault-injection --test $name -- \
        --test-threads=1 --nocapture; rm crates/bolt-vcs/tests/$name.rs"
    ;;
script)
    file=${2:?usage: run.sh script <file>}
    case $file in
    *.py) bash "$SANDBOX" run timeout 3000 python3 -I "redteam/attacks11/$file" "${@:3}" ;;
    *) bash "$SANDBOX" run timeout 3000 bash "redteam/attacks11/$file" "${@:3}" ;;
    esac
    ;;
*)
    sed -n '2,8p' "$0" >&2
    exit 2
    ;;
esac
