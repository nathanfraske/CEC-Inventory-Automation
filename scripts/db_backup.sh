#!/usr/bin/env bash
# Back up BOTH the database AND the receipt/photo object store to a directory OUTSIDE the repo.
# Both are secret data: never commit them. Default target /var/backups/cec-inventory (BACKUP_DIR).
#
# The DB dump only holds *references* to receipt files (purchase.receipt_files); the receipts
# themselves — the legal RMA proof-of-purchase artifacts (scope §7) — live in the object store.
# Backing up Postgres alone would leave a restore with dangling pointers, so we archive both.
#
# Optional env:
#   BACKUP_DIR              target dir (default /var/backups/cec-inventory)
#   BACKUP_AGE_RECIPIENT    age public key — when set (and `age` is installed), encrypt at rest
#   BACKUP_RETENTION_DAYS   prune backups older than this many days (default 30; 0 = keep all)
# Schedule it with scripts/systemd/cec-backup.{service,timer} (or cron). See SECRETS-AND-DATABASE.md.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck disable=SC1091
. ./.env
# shellcheck disable=SC1091
. ./scripts/_pglib.sh   # cec_pg_* + cec_objects_* (host pg tools, or the db container if absent)

BACKUP_DIR="${BACKUP_DIR:-/var/backups/cec-inventory}"
mkdir -p "$BACKUP_DIR"
chmod 700 "$BACKUP_DIR"
STAMP="$(date +%Y%m%dT%H%M%SZ)"
DB_OUT="${BACKUP_DIR}/cec_inventory_${STAMP}.dump"

# Encrypt a file at rest with age if BACKUP_AGE_RECIPIENT is set; otherwise leave it (and warn).
maybe_encrypt() {
  local f="$1"
  if [ -n "${BACKUP_AGE_RECIPIENT:-}" ]; then
    if command -v age >/dev/null 2>&1; then
      age -r "$BACKUP_AGE_RECIPIENT" -o "${f}.age" "$f"
      rm -f "$f"
      chmod 600 "${f}.age"
      echo "wrote ${f}.age (encrypted)"
    else
      echo "WARNING: BACKUP_AGE_RECIPIENT set but 'age' not installed — left $f UNENCRYPTED."
    fi
  else
    echo "wrote $f"
    echo "  (set BACKUP_AGE_RECIPIENT to encrypt at rest; replicate offsite if this is your only copy)"
  fi
}

# 1) Database — custom-format dump, restorable with pg_restore. Uses host pg tools, or the db
#    container when the host has none (CEC_PG_MODE).
echo "pg mode: ${CEC_PG_MODE}"
cec_pg_dump_custom "$DATABASE_URL" "$DB_OUT"
chmod 600 "$DB_OUT"
# Validate the dump is readable before claiming success (catches a partial/failed dump).
cec_pg_restore_list "$DB_OUT"
maybe_encrypt "$DB_OUT"

# 2) Object store (receipts/photos), timestamp-paired with the DB dump so they restore as a set.
#    Archives a host STORAGE_FS_ROOT, or the receipts named volume when they live in Docker.
OBJ_OUT="${BACKUP_DIR}/cec_objects_${STAMP}.tar.gz"
if cec_objects_archive "$OBJ_OUT"; then
  chmod 600 "$OBJ_OUT"
  maybe_encrypt "$OBJ_OUT"
else
  echo "WARNING: object store not found (no host STORAGE_FS_ROOT, no '$(cec_objects_volume)'"
  echo "  volume). The DB dump alone does NOT protect receipts (scope §7 RMA proof)."
fi

# 3) Retention: prune old backups (both .dump/.tar.gz and their .age variants).
RET="${BACKUP_RETENTION_DAYS:-30}"
if [ "$RET" -gt 0 ] 2>/dev/null; then
  find "$BACKUP_DIR" -maxdepth 1 -type f -name 'cec_*' -mtime "+${RET}" -print -delete \
    | sed 's/^/pruned /' || true
fi

# 4) Offsite replication (optional): copy THIS run's artifacts to a second location set in
#    BACKUP_OFFSITE_DIR — ideally a different failure domain (on WSL2 a /mnt/c path survives a
#    reset of the ext4 vdisk; the Windows drive may have its own backup). The .age files are
#    encrypted, so a permission-lossy/untrusted destination (e.g. NTFS) is acceptable for them.
#    NOTE: this does NOT replicate the age PRIVATE KEY. Keep that copied somewhere safe separately,
#    or the offsite archives are undecryptable after a disk loss (see SECRETS-AND-DATABASE.md §6).
if [ -n "${BACKUP_OFFSITE_DIR:-}" ]; then
  mkdir -p "$BACKUP_OFFSITE_DIR"
  if [ -z "${BACKUP_AGE_RECIPIENT:-}" ]; then
    echo "WARNING: replicating UNENCRYPTED backups to $BACKUP_OFFSITE_DIR (BACKUP_AGE_RECIPIENT unset)."
  fi
  copied=0
  for f in "$BACKUP_DIR"/cec_*_"${STAMP}".*; do
    [ -e "$f" ] || continue
    cp -f "$f" "$BACKUP_OFFSITE_DIR/" && copied=$((copied + 1))
  done
  echo "offsite: replicated $copied file(s) to $BACKUP_OFFSITE_DIR"
  # Mirror the retention policy to the offsite copy so it doesn't grow unbounded.
  if [ "${RET:-0}" -gt 0 ] 2>/dev/null; then
    find "$BACKUP_OFFSITE_DIR" -maxdepth 1 -type f -name 'cec_*' -mtime "+${RET}" -print -delete \
      | sed 's/^/offsite pruned /' || true
  fi
  # Opt-in: also mirror the age PRIVATE KEY so the offsite location is self-sufficient for
  # recovery (key + ciphertext together). This co-locates the key with the data it decrypts, so
  # enable ONLY on a destination you trust as much as the box. Keeps a rotated key in sync.
  if [ -n "${BACKUP_OFFSITE_INCLUDE_KEY:-}" ] \
     && [ -n "${BACKUP_AGE_IDENTITY:-}" ] && [ -f "$BACKUP_AGE_IDENTITY" ]; then
    cp -f "$BACKUP_AGE_IDENTITY" "${BACKUP_OFFSITE_DIR}/$(basename "$BACKUP_AGE_IDENTITY")"
    echo "offsite: replicated the age private key (BACKUP_OFFSITE_INCLUDE_KEY set)"
  fi
fi

echo "done. Verify restorability periodically with scripts/restore_drill.sh"
