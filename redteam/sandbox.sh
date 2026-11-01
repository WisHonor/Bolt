#!/usr/bin/env bash
# Red-team sandbox (decision 024).
#   redteam/sandbox.sh up <copy-dir>   copy HEAD into <copy-dir>, start the no-network container
#   redteam/sandbox.sh run <cmd...>    run a command inside it (working dir /work = the copy)
#   redteam/sandbox.sh down            stop and remove the container
set -euo pipefail
NAME=bolt-redteam
IMAGE=bolt-redteam:latest
CACHE=bolt-redteam-cargo
HERE=$(cd "$(dirname "$0")" && pwd)

case "${1:-}" in
up)
    copy=${2:?usage: sandbox.sh up <copy-dir>}
    [ -e "$copy" ] && { echo "refusing: $copy already exists" >&2; exit 1; }
    mkdir -p "$copy"
    git -C "$HERE/.." archive HEAD | tar -x -C "$copy"
    docker build -q -t "$IMAGE" "$HERE" >/dev/null
    # One networked step: download dependencies into the cache volume.
    MSYS_NO_PATHCONV=1 docker run --rm -v "$CACHE:/home/attacker/cargo" -v "$(cd "$copy" && pwd -W 2>/dev/null || pwd):/work" \
        -e CARGO_NET_OFFLINE=false "$IMAGE" cargo fetch --locked
    MSYS_NO_PATHCONV=1 docker run -d --name "$NAME" \
        --network none --memory 4g --memory-swap 4g --cpus 4 --pids-limit 1024 \
        --cap-drop ALL --security-opt no-new-privileges \
        -v "$CACHE:/home/attacker/cargo" -v "$(cd "$copy" && pwd -W 2>/dev/null || pwd):/work" \
        "$IMAGE" sleep infinity >/dev/null
    echo "sandbox up: container $NAME, copy at $copy, no network"
    ;;
run)
    shift
    MSYS_NO_PATHCONV=1 docker exec -w /work "$NAME" "$@"
    ;;
down)
    docker rm -f "$NAME" >/dev/null && echo "sandbox removed"
    ;;
*)
    sed -n '2,6p' "$0" >&2
    exit 2
    ;;
esac
