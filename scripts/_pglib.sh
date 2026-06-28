#!/usr/bin/env bash
# Shared Postgres-client + object-store routing for the backup/restore scripts.
# Source this AFTER `. ./.env` (it needs DATABASE_URL) and after `cd`-ing to the repo root.
#
# On a Docker-only box the host often has no psql/pg_dump/pg_restore and the receipts live in a
# named volume (not a host path). So:
#   * PG tools route through the `db` compose service when the host lacks them. Inside the network
#     the server is `db:5432`, so URLs are rewritten localhost->db. Dumps/restores stream over
#     stdin/stdout, so the actual files stay on the host in either mode.
#   * The object store is archived/extracted by streaming tar through a throwaway container bound
#     to the named volume (no host path or volume-name guess needed for the data itself).

# Rewrite the host[:port] of a postgres URL to db:5432 (the in-cluster address).
_pg_to_container_url() { printf '%s' "$1" | sed -E 's#(postgres(ql)?://[^@]*@)[^/?]+#\1db:5432#'; }

if command -v pg_dump >/dev/null 2>&1 && command -v pg_restore >/dev/null 2>&1 \
   && command -v psql >/dev/null 2>&1; then
  CEC_PG_MODE="host"
  cec_pg() { "$@"; }
  cec_pg_url() { printf '%s' "$1"; }
else
  CEC_PG_MODE="container"
  cec_pg() { docker compose exec -T db "$@"; }
  cec_pg_url() { _pg_to_container_url "$1"; }
fi

# pg_dump (custom format) of $1(url) -> $2(host file). Dump streams to stdout, redirected here.
cec_pg_dump_custom() { cec_pg pg_dump --format=custom --no-owner --dbname="$(cec_pg_url "$1")" >"$2"; }
# Validate a custom dump file is readable (catches a partial/corrupt dump).
cec_pg_restore_list() { cec_pg pg_restore --list <"$1" >/dev/null; }
# Restore custom dump $2(host file) into $1(url).
cec_pg_restore_into() { cec_pg pg_restore --no-owner --dbname="$(cec_pg_url "$1")" <"$2"; }
# Restore with --clean --if-exists into a live DB $1(url) from $2(host file).
cec_pg_restore_clean() {
  cec_pg pg_restore --clean --if-exists --no-owner --dbname="$(cec_pg_url "$1")" <"$2"
}
# psql against $1(url): cec_psql_c <url> <sql> [extra psql args...]
cec_psql_c() { local u="$1" sql="$2"; shift 2; cec_pg psql "$(cec_pg_url "$u")" "$@" -c "$sql"; }
# psql -tAc (tuples only): cec_psql_tac <url> <sql>
cec_psql_tac() { cec_pg psql "$(cec_pg_url "$1")" -tAc "$2"; }

# Name of the receipt object-store named volume (compose <project>_objects).
cec_objects_volume() {
  local proj
  proj="${COMPOSE_PROJECT_NAME:-}"
  [ -n "$proj" ] || proj="$(docker compose config --format json 2>/dev/null \
      | grep -oE '"name"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | sed -E 's/.*"([^"]*)"$/\1/')"
  [ -n "$proj" ] || proj="$(basename "$(pwd)")"
  printf '%s_objects' "$proj"
}

# Archive the object store to $1(host .tar.gz). Prefers a host STORAGE_FS_ROOT; else streams the
# named volume through a throwaway container (stdout-piped so the file is owned by the host user).
cec_objects_archive() {  # $1=out_file ; returns 1 if no object store found
  local out="$1" root="${STORAGE_FS_ROOT:-}" vol
  if [ -n "$root" ] && [ -d "$root" ]; then
    tar -czf "$out" -C "$root" .
    return 0
  fi
  vol="$(cec_objects_volume)"
  if docker volume inspect "$vol" >/dev/null 2>&1; then
    docker run --rm -v "${vol}:/data:ro" alpine tar -czf - -C /data . >"$out"
    return 0
  fi
  return 1
}

# Extract object archive $1(host .tar.gz) back into the store (host path or named volume).
cec_objects_extract() {  # $1=archive_file ; returns 1 if no destination found
  local arc="$1" root="${STORAGE_FS_ROOT:-}" vol
  if [ -n "$root" ] && [ -d "$(dirname "$root")" ]; then
    mkdir -p "$root"; tar -xzf "$arc" -C "$root"; return 0
  fi
  vol="$(cec_objects_volume)"
  if docker volume inspect "$vol" >/dev/null 2>&1; then
    docker run --rm -i -v "${vol}:/data" alpine tar -xzf - -C /data <"$arc"
    return 0
  fi
  return 1
}
