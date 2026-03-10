# Delta — multi-stage production build
# Stage 1: Build
FROM rust:1.85-bookworm AS builder

RUN apt-get update && apt-get install -y \
    cmake pkg-config libssl-dev git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .
RUN cargo build --release --package delta-api

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates git tini \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd -r delta && useradd -r -g delta -u 1000 delta

RUN mkdir -p /var/lib/delta/repos \
             /var/lib/delta/artifacts \
             /var/lib/delta/data \
             /etc/delta \
    && chown -R delta:delta /var/lib/delta /etc/delta

COPY --from=builder /build/target/release/delta-api /usr/local/bin/delta-server
COPY config/delta.example.toml /etc/delta/config.toml

USER delta
EXPOSE 8070

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -sf http://localhost:8070/health || exit 1

ENTRYPOINT ["tini", "--"]
CMD ["delta-server", "--config", "/etc/delta/config.toml"]
