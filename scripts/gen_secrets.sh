#!/usr/bin/env bash
# Generate the DB password and session secret into .env (gitignored).
# Re-running rotates both. Never prints the secret values.
set -eu
cd "$(dirname "$0")/.."

if [ ! -f .env ]; then
  cp .env.example .env
  echo "created .env from .env.example"
fi

# URL-safe alnum password (strip /+= so it is safe in a connection string and in sed)
PW="$(openssl rand -base64 32 | tr -dc 'A-Za-z0-9' | cut -c1-32)"
SS="$(openssl rand -hex 32)"

sed -i "s|^POSTGRES_PASSWORD=.*|POSTGRES_PASSWORD=${PW}|" .env
sed -i "s|^SESSION_SECRET=.*|SESSION_SECRET=${SS}|" .env

# Rebuild DATABASE_URL from the parts now in .env
# shellcheck disable=SC1091
. ./.env
sed -i "s|^DATABASE_URL=.*|DATABASE_URL=postgres://${POSTGRES_USER}:${PW}@${POSTGRES_HOST}:${POSTGRES_PORT}/${POSTGRES_DB}|" .env

chmod 600 .env
echo "secrets written to .env (chmod 600, gitignored). Do NOT commit this file."
echo "if this repo was ever pushed with a real secret, rotate it; see SECRETS-AND-DATABASE.md."
