#!/usr/bin/env bash
# The rename red team, session RT15 (decision 024). Never run on the host.
#   redteam/attacks14/run.sh probe [<name>...]   probe.py's probes of the release program
#   redteam/attacks14/run.sh explore <name>      one exploration file of this folder (rt15_*.rs),
#                                                copied into crates/opal-vcs/tests/ for the run
#   redteam/attacks14/run.sh platforms           lints redteam15_platforms.rs with its Windows
#                                                and macOS gates taken off (it can't run here)
# Probes use /home/attacker/target/release/opalro: build it first
# (CARGO_TARGET_DIR=/home/attacker/target cargo build --release -p opal-vcs).
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
SANDBOX="$HERE/../sandbox.sh"
T=CARGO_TARGET_DIR=/home/attacker/target
case "${1:-}" in
probe)
    bash "$SANDBOX" run timeout 3000 python3 redteam/attacks14/probe.py "${@:2}"
    ;;
explore)
    name=${2:?usage: run.sh explore <rt15_name>}
    bash "$SANDBOX" run sh -c "cp redteam/attacks14/$name.rs crates/opal-vcs/tests/ && \
        $T timeout 2400 cargo test -j 4 --release -p opal-vcs --test $name -- \
        --test-threads=1 --nocapture; rm crates/opal-vcs/tests/$name.rs"
    ;;
platforms)
    bash "$SANDBOX" run sh -c "sed -e 's/^#!\[cfg(any(windows, target_os = \"macos\"))\]//' \
        -e 's/^#\[cfg(windows)\]//' crates/opal-vcs/tests/redteam15_platforms.rs \
        > crates/opal-vcs/tests/rt15_platcheck.rs && \
        $T cargo clippy -q --release -p opal-vcs --test rt15_platcheck -- -D warnings; \
        s=\$?; rm crates/opal-vcs/tests/rt15_platcheck.rs; exit \$s"
    ;;
*)
    sed -n '2,9p' "$0" >&2
    exit 2
    ;;
esac
