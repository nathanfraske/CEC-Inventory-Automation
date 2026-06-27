#!/usr/bin/env bash
# Dump the database to a directory OUTSIDE the repo. Dumps are secret data:
# never commit them. Default target is /var/backups/cec-inventory (override with BACKUP_DIR).
set -eu
cd "$(dirname "$0")/.."
# shellcheck disable=SC1091
. ./.env

BACKUP_DIR="${BACKUP_DIR:-/var/backups/cec-inventory}"
mkdir -p "$BACKUP_DIR"
chmod 700 "$BACKUP_DIR"
STAMP="$(date +%Y%m%dT%H%M%SZ)"
OUT="${BACKUP_DIR}/cec_inventory_${STAMP}.dump"

# Custom format dump, restorable with pg_restore. Uses DATABASE_URL.
pg_dump --format=custom --no-owner --dbname="$DATABASE_URL" --file="$OUT"
chmod 600 "$OUT"
echo "wrote $OUT"
echo "store offsite if this is your only copy. Do not place backups in the repo."
