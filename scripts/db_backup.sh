#!/usr/bin/env bash
# Back up BOTH the database AND the receipt/photo object store to a directory OUTSIDE the repo.
# Both are secret data: never commit them. Default target /var/backups/cec-inventory (BACKUP_DIR).
#
# The DB dump only holds *references* to receipt files (purchase.receipt_files); the receipts
# themselves — the legal RMA proof-of-purchase artifacts (scope §7) — live in the object store.
# Backing up Postgres alone would leave a restore with dangling pointers, so we archive both.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck disable=SC1091
. ./.env

BACKUP_DIR="${BACKUP_DIR:-/var/backups/cec-inventory}"
mkdir -p "$BACKUP_DIR"
chmod 700 "$BACKUP_DIR"
STAMP="$(date +%Y%m%dT%H%M%SZ)"
DB_OUT="${BACKUP_DIR}/cec_inventory_${STAMP}.dump"

# 1) Database — custom-format dump, restorable with pg_restore. Uses DATABASE_URL.
pg_dump --format=custom --no-owner --dbname="$DATABASE_URL" --file="$DB_OUT"
chmod 600 "$DB_OUT"
# Validate the dump is readable before claiming success (catches a partial/failed dump).
pg_restore --list "$DB_OUT" >/dev/null
echo "wrote $DB_OUT"

# 2) Object store (receipts/photos), timestamp-paired with the DB dump so they restore as a set.
OBJ_ROOT="${STORAGE_FS_ROOT:-}"
OBJ_OUT="${BACKUP_DIR}/cec_objects_${STAMP}.tar.gz"
if [ -n "$OBJ_ROOT" ] && [ -d "$OBJ_ROOT" ]; then
  tar -czf "$OBJ_OUT" -C "$OBJ_ROOT" .
  chmod 600 "$OBJ_OUT"
  echo "wrote $OBJ_OUT"
else
  echo "WARNING: object store '$OBJ_ROOT' not found on this host."
  echo "  If receipts live in the Docker named volume 'objects', archive it with:"
  echo "    docker run --rm -v cec-inventory_objects:/data -v \"$BACKUP_DIR\":/out alpine \\"
  echo "      tar -czf /out/cec_objects_${STAMP}.tar.gz -C /data ."
  echo "  The DB dump alone does NOT protect receipts (scope §7 RMA proof) — do not skip this."
fi

echo "Encrypt (age/gpg) and replicate offsite if this is your only copy. Never place backups in the repo."
echo "Restore both together: scripts/db_restore.sh $DB_OUT $OBJ_OUT"
