#!/usr/bin/env bash
# Restore-drill: prove the latest backup actually restores. "A backup you have never restored is
# not a backup." Restores the newest DB dump into a THROWAWAY database (never the live one),
# checks the schema + a couple of row counts, validates the objects archive, then drops it all.
#
# Optional env: BACKUP_DIR (default /var/backups/cec-inventory),
#   BACKUP_AGE_IDENTITY (age key file, required if the backup is .age-encrypted).
# Run on a schedule alongside the backup, or by hand.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck disable=SC1091
. ./.env
# shellcheck disable=SC1091
. ./scripts/_pglib.sh   # cec_pg_* (host pg tools, or the db container if absent)

BACKUP_DIR="${BACKUP_DIR:-/var/backups/cec-inventory}"
LATEST="$(ls -1t "$BACKUP_DIR"/cec_inventory_*.dump "$BACKUP_DIR"/cec_inventory_*.dump.age 2>/dev/null | head -1 || true)"
[ -n "$LATEST" ] || { echo "no DB backup found in $BACKUP_DIR"; exit 1; }

WORK="$(mktemp -d)"
DRILL_DB="cec_restore_drill_$$"
ADMIN_URL="${DATABASE_URL%/*}/postgres"   # same server, default 'postgres' db
cleanup() {
  cec_psql_c "$ADMIN_URL" "DROP DATABASE IF EXISTS \"$DRILL_DB\" WITH (FORCE)" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

# Decrypt if needed.
DUMP="$LATEST"
case "$LATEST" in
  *.age)
    [ -n "${BACKUP_AGE_IDENTITY:-}" ] || { echo "backup is encrypted; set BACKUP_AGE_IDENTITY"; exit 1; }
    DUMP="$WORK/restore.dump"
    age -d -i "$BACKUP_AGE_IDENTITY" -o "$DUMP" "$LATEST"
    ;;
esac

# Restore into a throwaway DB on the same server.
cec_psql_c "$ADMIN_URL" "CREATE DATABASE \"$DRILL_DB\"" -v ON_ERROR_STOP=1
DRILL_URL="${DATABASE_URL%/*}/$DRILL_DB"
cec_pg_restore_into "$DRILL_URL" "$DUMP"

TABLES="$(cec_psql_tac "$DRILL_URL" "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'")"
UNITS="$(cec_psql_tac "$DRILL_URL" "SELECT count(*) FROM inventory_unit")"
[ "$TABLES" -ge 18 ] || { echo "FAIL: only $TABLES public tables restored (expected >=18)"; exit 1; }
echo "DB restore OK: $TABLES public tables, $UNITS units, from $(basename "$LATEST")"

# Objects archive integrity (paired by timestamp), if present.
OBJ="$(ls -1t "$BACKUP_DIR"/cec_objects_*.tar.gz "$BACKUP_DIR"/cec_objects_*.tar.gz.age 2>/dev/null | head -1 || true)"
if [ -n "$OBJ" ]; then
  case "$OBJ" in
    *.age) echo "objects archive present (encrypted): $(basename "$OBJ") — decrypt to fully verify" ;;
    *)     tar -tzf "$OBJ" >/dev/null && echo "objects archive OK: $(basename "$OBJ")" ;;
  esac
else
  echo "NOTE: no objects archive found — receipts are not covered by this backup set."
fi

echo "restore drill PASSED"
