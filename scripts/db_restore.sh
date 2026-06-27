#!/usr/bin/env bash
# Restore a custom-format dump into the database. Destructive: confirm first.
set -eu
cd "$(dirname "$0")/.."
# shellcheck disable=SC1091
. ./.env

FILE="${1:-}"
if [ -z "$FILE" ]; then echo "usage: $0 <dump-file>"; exit 1; fi
if [ ! -f "$FILE" ]; then echo "no such file: $FILE"; exit 1; fi

printf "restore %s into %s ? this overwrites current data [y/N] " "$FILE" "$POSTGRES_DB"
read -r ans
[ "$ans" = "y" ] || { echo "aborted"; exit 1; }

pg_restore --clean --if-exists --no-owner --dbname="$DATABASE_URL" "$FILE"
echo "restore complete"
