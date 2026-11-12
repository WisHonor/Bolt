# Red-team sandbox (decision 024): Opalro and attack code run here, never on the host.
FROM rust:1.99.1-slim-bookworm
RUN rustup component add rustfmt clippy \
    && apt-get update && apt-get install -y --no-install-recommends git git-lfs python3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home attacker \
    && mkdir -p /work /home/attacker/cargo \
    && chown attacker /work /home/attacker/cargo
USER attacker
# The registry cache is filled once with network, then the sandbox runs offline.
ENV CARGO_HOME=/home/attacker/cargo CARGO_NET_OFFLINE=true PATH=/usr/local/cargo/bin:$PATH
WORKDIR /work
