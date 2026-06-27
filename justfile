# Common tasks. Run `just` for the list. Requires `just` (cargo install just).

set dotenv-load := true

# Generate DB password and session secret into .env (gitignored)
secrets:
    ./scripts/gen_secrets.sh

# Bring up Postgres (and the api once built)
up:
    docker compose up -d db
    @echo "Postgres up on 127.0.0.1:${POSTGRES_PORT}"

down:
    docker compose down

# Run migrations (needs sqlx-cli: cargo install sqlx-cli --no-default-features --features rustls,postgres)
migrate:
    sqlx migrate run

# Regenerate the SQLx offline cache so CI builds without a live DB
prepare:
    cargo sqlx prepare --workspace

# Build and run the api locally
run:
    cargo run -p cec-inventory-api

fmt:
    cargo fmt --all

lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Scan the working tree for secrets before committing
scan:
    gitleaks detect --source . --redact

backup:
    ./scripts/db_backup.sh

restore FILE:
    ./scripts/db_restore.sh {{FILE}}
