#!/bin/sh
# RT17's attack code. From the copy's top folder:
#   bash redteam/sandbox.sh run sh redteam/attacks16/run.sh proofs    # the five findings, each should fail
#   bash redteam/sandbox.sh run sh redteam/attacks16/run.sh explore   # the probes, printing what each command said
#   bash redteam/sandbox.sh run sh redteam/attacks16/run.sh walk      # only_in against brute force, 15,000 queries
set -e
tests=crates/opal-vcs/tests
case "$1" in
proofs)
    cargo test -p opal-vcs --all-features --test redteam17 -- --ignored
    ;;
explore | walk)
    name=rt17_$1
    cp "redteam/attacks16/$name.rs" "$tests/$name.rs"
    trap 'rm -f "$tests/$name.rs"' EXIT
    cargo test --release -p opal-vcs --all-features --test "$name" -- --nocapture --test-threads=1
    ;;
*)
    echo "usage: run.sh proofs|explore|walk" >&2
    exit 2
    ;;
esac
