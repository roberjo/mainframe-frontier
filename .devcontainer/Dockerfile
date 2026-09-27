# Mainframe Frontier dev image: Rust toolchain + GnuCOBOL 3.2.
# (Node 24 for the Control Room and MCP server is added in Phase 3.)
# Used by the devcontainer, docker compose, and CI so every environment
# compiles and runs the COBOL workload identically.
FROM rust:1.97-slim-trixie

RUN apt-get update \
 && apt-get install -y --no-install-recommends gnucobol3 make jq \
 && rm -rf /var/lib/apt/lists/* \
 && rustup component add clippy rustfmt

ENV FRONTIER_HOME=/workspace/var \
    CARGO_TARGET_DIR=/cargo-target \
    PATH=/cargo-target/release:$PATH

WORKDIR /workspace
