#!/usr/bin/env bash
# The v0.1 release red team, session RT14 (decision 024). Never run on the host.
#   redteam/attacks13/run.sh proofs           the proof tests, redteam14.rs (release build)
#   redteam/attacks13/run.sh explore <name>   one exploration file of this folder (rt14_*.rs),
#                                             copied into crates/bolt-vcs/tests/ for the run
#   redteam/attacks13/run.sh script <file>    one of the shell probes of this folder
# Scripts use /work/target/release/bolt: build it first (cargo build --release -p bolt-vcs).
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
SANDBOX="$HERE/../sandbox.sh"
case "${1:-}" in
proofs)
    bash "$SANDBOX" run sh -c 'cargo test -j 4 --release -p bolt-vcs --test redteam14 -- \
        --ignored --test-threads=1 --nocapture'
    ;;
explore)
    name=${2:?usage: run.sh explore <rt14_name>}
    bash "$SANDBOX" run sh -c "cp redteam/attacks13/$name.rs crates/bolt-vcs/tests/ && \
        timeout 2400 cargo test -j 4 --release -p bolt-vcs --test $name -- \
        --test-threads=1 --nocapture; rm crates/bolt-vcs/tests/$name.rs"
    ;;
script)
    file=${2:?usage: run.sh script <file>}
    bash "$SANDBOX" run timeout 3000 bash "redteam/attacks13/$file" "${@:3}"
    ;;
*)
    sed -n '2,7p' "$0" >&2
    exit 2
    ;;
esac
