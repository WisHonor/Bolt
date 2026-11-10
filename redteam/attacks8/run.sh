#!/usr/bin/env bash
# Milestone 5 red team, session 2 (decision 024). Never run on the host.
#   redteam/attacks8/run.sh proofs           the proof tests, redteam9.rs
#   redteam/attacks8/run.sh explore <name>   one exploration file of this folder (rt9_*.rs),
#                                            copied into crates/bolt-vcs/tests/ for the run
#   redteam/attacks8/run.sh script <file>    one of the shell or Python probes of this folder
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
SANDBOX="$HERE/../sandbox.sh"
case "${1:-}" in
proofs)
    bash "$SANDBOX" run sh -c 'cargo test -j 2 -p bolt-vcs --features fault-injection \
        --test redteam9 -- --ignored --test-threads=2'
    ;;
explore)
    name=${2:?usage: run.sh explore <rt9_name>}
    bash "$SANDBOX" run sh -c "cp redteam/attacks8/$name.rs crates/bolt-vcs/tests/ && \
        cargo test -j 2 -p bolt-vcs --features fault-injection --test $name -- \
        --test-threads=1 --nocapture; rm crates/bolt-vcs/tests/$name.rs"
    ;;
script)
    file=${2:?usage: run.sh script <file>}
    case $file in
    *.py) bash "$SANDBOX" run python3 -I "redteam/attacks8/$file" ;;
    *) bash "$SANDBOX" run bash "redteam/attacks8/$file" "${@:3}" ;;
    esac
    ;;
*)
    sed -n '2,6p' "$0" >&2
    exit 2
    ;;
esac
