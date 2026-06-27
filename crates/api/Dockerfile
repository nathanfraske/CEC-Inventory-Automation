# Multi-stage build for the api. SQLX_OFFLINE lets the build skip a live DB.
FROM rust:1-bookworm AS build
WORKDIR /src
ENV SQLX_OFFLINE=true
COPY . .
RUN cargo build --release -p cec-inventory-api

FROM debian:bookworm-slim
# curl is used by the compose healthcheck (GET /readyz); ca-certificates for outbound TLS.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
# Run as a non-root user. Create + own the object-store dir so the named volume inherits the
# ownership on first creation (the api also create_dir_all's it on startup).
RUN useradd -r -u 10001 -m -d /home/cec cec \
    && mkdir -p /var/lib/cec-inventory/objects \
    && chown -R cec:cec /var/lib/cec-inventory
COPY --from=build /src/target/release/cec-inventory-api /usr/local/bin/cec-inventory-api
USER cec
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/cec-inventory-api"]
