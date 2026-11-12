#!/usr/bin/env bash
# Red-team session RT16, the final builds (decision 024). Never run on the host.
#   redteam/attacks15/run.sh proofs      redteam16.rs, the proof tests, run with --ignored
#   redteam/attacks15/run.sh explore     rt16_explore.rs: hostile frames through both read paths,
#                                        a child of pack.rs for the run
#   redteam/attacks15/run.sh cli         rt16_cli.rs: planted frames through the program
#   redteam/attacks15/run.sh names       rt16_names.rs (the rule against the spec and Git's form),
#                                        then names.sh through the release program
#   redteam/attacks15/run.sh platforms   lints redteam16_platforms.rs, the FAT probe (S1), which
#                                        needs macOS or a mounted FAT folder in RT16_FAT_DIR
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
SANDBOX="$HERE/../sandbox.sh"
A=redteam/attacks15
case "${1:-}" in
proofs)
    bash "$SANDBOX" run sh -c "cp $A/redteam16.rs crates/opal-vcs/tests/ && \
        timeout 2400 cargo test -j 2 --release -p opal-vcs --test redteam16 -- --ignored \
        --test-threads=2; s=\$?; rm crates/opal-vcs/tests/redteam16.rs; exit \$s"
    ;;
explore)
    P=crates/opal-vcs-core/src/pack.rs
    bash "$SANDBOX" run sh -c "cp $P /tmp/pack.rs && cp $A/rt16_explore.rs crates/opal-vcs-core/src/ && \
        printf '\n#[cfg(test)]\n#[path = \"rt16_explore.rs\"]\nmod rt16_explore;\n' >> $P && \
        timeout 2400 cargo test -j 2 --release -p opal-vcs-core --lib rt16_explore -- \
        --nocapture --test-threads=1; s=\$?; cp /tmp/pack.rs $P; \
        rm crates/opal-vcs-core/src/rt16_explore.rs; exit \$s"
    ;;
cli)
    bash "$SANDBOX" run sh -c "cp $A/rt16_cli.rs crates/opal-vcs/tests/ && \
        timeout 2400 cargo test -j 2 --release -p opal-vcs --test rt16_cli -- --nocapture \
        --test-threads=1; s=\$?; rm crates/opal-vcs/tests/rt16_cli.rs; exit \$s"
    ;;
names)
    bash "$SANDBOX" run sh -c "cp $A/rt16_names.rs crates/opal-vcs-core/tests/ && \
        timeout 2400 cargo test -j 2 --release -p opal-vcs-core --test rt16_names -- \
        --nocapture; s=\$?; rm crates/opal-vcs-core/tests/rt16_names.rs; \
        cargo build -j 2 --release -p opal-vcs && bash $A/names.sh; exit \$s"
    ;;
platforms)
    bash "$SANDBOX" run sh -c "cp $A/redteam16_platforms.rs crates/opal-vcs/tests/ && \
        cargo clippy -j 2 -q --release -p opal-vcs --test redteam16_platforms -- -D warnings; \
        s=\$?; rm crates/opal-vcs/tests/redteam16_platforms.rs; exit \$s"
    ;;
*)
    sed -n '2,11p' "$0" >&2
    exit 2
    ;;
esac
