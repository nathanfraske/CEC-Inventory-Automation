#!/usr/bin/env bash
# Restore a custom-format DB dump (and, optionally, the paired object-store archive).
# Destructive: confirm first. Restore into a stopped/empty target — the app provisions the
# schema via sqlx::migrate! on first start, so prefer: stop the API, restore, then start.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck disable=SC1091
. ./.env
# shellcheck disable=SC1091
. ./scripts/_pglib.sh   # cec_pg_* + cec_objects_* (host pg tools, or the db container if absent)

FILE="${1:-}"
OBJ_FILE="${2:-}"
if [ -z "$FILE" ]; then
  echo "usage: $0 <db-dump-file> [objects-archive.tar.gz]"
  exit 1
fi
if [ ! -f "$FILE" ]; then echo "no such file: $FILE"; exit 1; fi

printf "restore %s into %s ? this overwrites current data [y/N] " "$FILE" "$POSTGRES_DB"
read -r ans
[ "$ans" = "y" ] || { echo "aborted"; exit 1; }

cec_pg_restore_clean "$DATABASE_URL" "$FILE"
echo "database restore complete"

# Receipts/photos: restore the paired archive so receipt_files references resolve again.
if [ -n "$OBJ_FILE" ]; then
  if [ ! -f "$OBJ_FILE" ]; then echo "no such objects archive: $OBJ_FILE"; exit 1; fi
  if cec_objects_extract "$OBJ_FILE"; then
    echo "object store restored"
  else
    echo "could not restore objects (no host STORAGE_FS_ROOT and no objects volume)"; exit 1
  fi
else
  echo "NOTE: no objects archive given — receipts/photos were NOT restored. Pass it as arg 2"
  echo "      or the restored DB will reference receipt files that no longer exist."
fi
