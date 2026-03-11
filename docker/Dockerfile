# Delta — multi-stage production build on AGNOS
# Stage 1: Build
FROM rust:1.85-bookworm AS builder

RUN apt-get update && apt-get install -y \
    cmake pkg-config libssl-dev git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .
RUN cargo build --release --package delta-api

# Stage 2: Runtime on AGNOS
FROM ghcr.io/maccracken/agnosticos:latest

USER root

RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd -g 1003 delta && useradd -u 1003 -g delta -d /var/lib/delta -s /usr/sbin/nologin delta

RUN mkdir -p /var/lib/delta/repos \
             /var/lib/delta/artifacts \
             /var/lib/delta/data \
             /etc/delta \
    && chown -R delta:delta /var/lib/delta /etc/delta

COPY --from=builder /build/target/release/delta-api /usr/local/bin/delta-server
COPY config/delta.docker.toml /etc/delta/config.toml

USER delta
WORKDIR /var/lib/delta
EXPOSE 8070

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -sf http://localhost:8070/health || exit 1

ENTRYPOINT ["tini", "--"]
CMD ["delta-server", "--config", "/etc/delta/config.toml"]
