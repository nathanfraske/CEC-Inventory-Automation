# Multi-stage build for the shipment-tracking poller. SQLX_OFFLINE lets the build skip a
# live DB (the poller uses runtime queries; no .sqlx cache needed yet — see DECISIONS D-010).
FROM rust:1-bookworm AS build
WORKDIR /src
# CARGO_* vars harden crate downloads against the intermittent HTTP/2-framing flake on crates.io.
ENV SQLX_OFFLINE=true CARGO_NET_RETRY=5 CARGO_HTTP_MULTIPLEXING=false
COPY . .
RUN cargo build --release -p cec-inventory-poller

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN useradd -r -u 10001 cec
COPY --from=build /src/target/release/cec-inventory-poller /usr/local/bin/cec-inventory-poller
USER cec
ENTRYPOINT ["/usr/local/bin/cec-inventory-poller"]
