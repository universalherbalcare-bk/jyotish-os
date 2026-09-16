# syntax=docker/dockerfile:1.7
# jyotish-mcp — L4 consensus/MCP server, containerized (Phase 8). Build context = repo root.
#
# Targets:
#   mcp        (default) the server: uid 10001, debian-slim, TLS on, kernels/ certs/ mounted read-only,
#              audit/ mounted read-write; published by compose as 127.0.0.1:7791 only.
#   consensus  the differential corpus runner (validation/consensus) for
#              `docker compose --profile corpus run --rm consensus-corpus` inside the sidecar network.
#
# XALEN crates are linked by path from vendor/xalen exactly as the native build does (no registry
# dependency); Cargo.lock is honoured with --locked. aws-lc-rs (rustls provider) needs cmake + a C
# compiler at build time only.

FROM rust:1.97-slim-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential cmake pkg-config clang libclang-dev perl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
# The xalen workspace manifest is read for `version.workspace = true`; every listed member must exist.
COPY vendor/xalen/Cargo.toml vendor/xalen/Cargo.lock vendor/xalen/
COPY vendor/xalen/crates vendor/xalen/crates
COPY vendor/xalen/validation vendor/xalen/validation
COPY services/jyotish-mcp services/jyotish-mcp
COPY validation/consensus validation/consensus
ENV CARGO_TERM_COLOR=never CARGO_NET_RETRY=5
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/services/jyotish-mcp/target,sharing=locked \
    cd services/jyotish-mcp && cargo build --release --locked \
    && install -m 0755 target/release/jyotish-mcp /usr/local/bin/jyotish-mcp
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/validation/consensus/target,sharing=locked \
    cd validation/consensus && cargo build --release --locked \
    && install -m 0755 target/release/consensus-corpus /usr/local/bin/consensus-corpus

FROM debian:bookworm-slim AS mcp
# curl only for the container HEALTHCHECK (https on loopback with the local CA).
RUN apt-get update && apt-get install -y --no-install-recommends curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -g 10001 jyotish \
    && useradd -u 10001 -g 10001 -M -d /nonexistent -s /usr/sbin/nologin jyotish \
    && mkdir -p /kernels /certs /audit
COPY --from=build --chown=root:root /usr/local/bin/jyotish-mcp /usr/local/bin/jyotish-mcp
ENV JYOTISH_DEPLOYMENT=compose \
    JYOTISH_BIND=0.0.0.0:7791 \
    JYOTISH_KERNEL=/kernels/de440s.bsp \
    JYOTISH_KERNEL_SHA256=/kernels/de440s.sha256 \
    JHORA_URL=http://jhora-svc:7792 \
    VEDASTRO_URL=http://vedastro-svc:7793 \
    JYOTISH_AUDIT=/audit/jyotish-mcp.jsonl \
    JYOTISH_TLS=on \
    JYOTISH_TLS_CERT=/certs/jyotish-mcp.crt \
    JYOTISH_TLS_KEY=/certs/jyotish-mcp.key
USER 10001:10001
WORKDIR /
EXPOSE 7791
HEALTHCHECK --interval=10s --timeout=5s --start-period=60s --retries=3 \
  CMD ["curl", "-sf", "--cacert", "/certs/jyotish-ca.crt", "https://127.0.0.1:7791/health"]
ENTRYPOINT ["/usr/local/bin/jyotish-mcp"]

FROM debian:bookworm-slim AS consensus
RUN groupadd -g 10001 jyotish \
    && useradd -u 10001 -g 10001 -M -d /nonexistent -s /usr/sbin/nologin jyotish \
    && mkdir -p /kernels /out && chown 10001:10001 /out
COPY --from=build --chown=root:root /usr/local/bin/consensus-corpus /usr/local/bin/consensus-corpus
ENV JYOTISH_DEPLOYMENT=compose
USER 10001:10001
WORKDIR /out
ENTRYPOINT ["/usr/local/bin/consensus-corpus"]
CMD ["--n", "500", "--seed", "42", "--jhora-url", "http://jhora-svc:7792", \
     "--kernel", "/kernels/de440s.bsp", "--sha", "/kernels/de440s.sha256", \
     "--csv", "/out/compose-consensus-corpus.csv", "--summary", "/out/compose-consensus-summary.md"]
