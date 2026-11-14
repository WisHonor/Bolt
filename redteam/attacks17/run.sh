#!/bin/sh
# RT18, inside the sandbox only: bash redteam/sandbox.sh run sh redteam/attacks17/run.sh <proofs|explore>
# explore copies the probes (kept here uncompiled) into the tests folder, runs them, and removes them.
set -eu
case "${1:-proofs}" in
proofs)
    cargo test -j 2 -p opal-vcs --all-features --test redteam18 -- --ignored --test-threads 1
    ;;
explore)
    for f in rt18_explore rt18_explore2; do
        cp "redteam/attacks17/$f.rs.txt" "crates/opal-vcs/tests/$f.rs"
    done
    trap 'rm -f crates/opal-vcs/tests/rt18_explore.rs crates/opal-vcs/tests/rt18_explore2.rs' EXIT
    cargo test -j 2 -p opal-vcs --all-features --test rt18_explore --test rt18_explore2 -- --nocapture --test-threads 1
    ;;
esac
