# Multi-stage build for the shipment-tracking poller. SQLX_OFFLINE lets the build skip a
# live DB (the poller uses runtime queries; no .sqlx cache needed yet — see DECISIONS D-010).
FROM rust:1-bookworm AS build
WORKDIR /src
ENV SQLX_OFFLINE=true
COPY . .
RUN cargo build --release -p cec-inventory-poller

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN useradd -r -u 10001 cec
COPY --from=build /src/target/release/cec-inventory-poller /usr/local/bin/cec-inventory-poller
USER cec
ENTRYPOINT ["/usr/local/bin/cec-inventory-poller"]
