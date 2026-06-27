# Secrets and the Database: Keeping Them Out of the Repo

Version: 1.0.0
Status: working basis.

Two separate problems, both solved here:
1. Keep database credentials (the password, the connection string, API keys) out of version
   control.
2. Keep the database data itself out of the repo (no data dir in the tree, no dumps committed).

The rule under everything: the repo is public-shaped. Assume anything committed could leak.
Real secret material lives only in places git never sees.

---

## 1. What counts as a secret here

- `POSTGRES_PASSWORD`
- `DATABASE_URL` (contains the password, so it is a secret in full)
- `SESSION_SECRET`
- `CARRIER_API_KEY` / `CARRIER_API_SECRET` (when a tracking provider is chosen, INV-OQ-30)
- MinIO access/secret keys (if object storage flips to MinIO)
- Any TLS private key, age key, or SOPS key

None of these belongs in a committed file. All of them live in `.env` (dev) or a host secret
store (prod), both described below.

---

## 2. The .gitignore strategy

`.gitignore` is the first file in the repo (Phase A of the runbook) so nothing secret can be
staged before it exists. It ignores, among others:

- `.env`, `*.env`, `secrets/`, `*.key`, `*.pem`, `*.agekey` (credentials)
- `pgdata/`, `db_data/`, `/data/`, `minio-data/` (live database data dirs)
- `*.dump`, `*.sql.gz`, `*.sqlite`, `backups/`, `dumps/` (database dumps)

It explicitly does NOT ignore `.env.example` (placeholders, committed) or `.sqlx/` (query
metadata, no secrets, committed for offline builds).

Verify on any machine: `git check-ignore .env` must print `.env`. If it does not, stop and
fix `.gitignore` before committing anything.

---

## 3. The .env workflow (development)

- `.env.example` is committed and carries only placeholders (`CHANGE_ME_*`, empty keys).
- `.env` is created from it and filled with real values by `scripts/gen_secrets.sh`, which:
  - copies `.env.example` to `.env` if absent,
  - writes a strong random `POSTGRES_PASSWORD` (URL-safe alnum, 32 chars) and a 64-hex
    `SESSION_SECRET`,
  - rebuilds `DATABASE_URL` from the parts,
  - `chmod 600 .env`.
- Re-running `gen_secrets.sh` rotates both secrets.
- Docker Compose auto-loads `.env`, so `docker compose` and the api read the same values.
  The Rust code reads `DATABASE_URL` from the environment and never hardcodes it.

Never echo `.env` into logs, issues, or chat. Never paste a real `DATABASE_URL` anywhere
git-tracked.

---

## 4. SQLx offline: build and CI without the DB or its secret

SQLx checks queries at compile time. To avoid needing a live database (and therefore the
secret) during builds:

- Run `cargo sqlx prepare --workspace` after any query change. It writes `.sqlx/` query
  metadata, which is committed (no secrets in it).
- Set `SQLX_OFFLINE=true` for CI and container builds (already set in `.github/workflows/ci.yml`
  and the api `Dockerfile`). Builds then use the committed cache, so CI never needs
  `DATABASE_URL` and no secret is exposed to the build environment.

This is the mechanism that lets the repo build in CI with zero database access.

---

## 5. Keeping the database DATA out of the repo

- Postgres runs in a Docker named volume (`pgdata`), declared in `docker-compose.yml`. A named
  volume lives in Docker's storage, not in the repo working tree, so the data cannot be staged.
- Do not use a bind mount that points inside the repo for the data dir. If you ever need a bind
  mount, point it outside the repo (for example `/var/lib/cec-inventory/pgdata`).
- Backups (`scripts/db_backup.sh`) write to a directory OUTSIDE the repo (default
  `/var/backups/cec-inventory`, override with `BACKUP_DIR`). Dumps are secret data: store them
  with 600 perms, offsite if they are the only copy, and never in the repo. The `.gitignore`
  already blocks `*.dump`, `*.sql.gz`, and `backups/` as a backstop.

---

## 6. Secrets in production (pick one)

Development uses `.env`. Production should not rely on a dotfile sitting in a repo checkout.
Three options, in increasing robustness:

### 6a. Env file outside the repo (simplest, fine for a single box)
Put the env file at a host path the repo never contains, owned root:root, mode 600:
`/etc/cec-inventory/inventory.env`. Point the runtime at it:
- Compose: `docker compose --env-file /etc/cec-inventory/inventory.env up -d`
- systemd unit: `EnvironmentFile=/etc/cec-inventory/inventory.env`

### 6b. Docker secrets for the DB password (better isolation)
The Postgres image reads `POSTGRES_PASSWORD_FILE`, so the password can be a mounted secret file
instead of an env var. Sketch:
```yaml
services:
  db:
    image: postgres:16
    environment:
      POSTGRES_USER: ${POSTGRES_USER}
      POSTGRES_DB: ${POSTGRES_DB}
      POSTGRES_PASSWORD_FILE: /run/secrets/db_password
    secrets: [db_password]
secrets:
  db_password:
    file: /etc/cec-inventory/db_password   # outside the repo, mode 600
```
The api then reads the same secret file to build `DATABASE_URL`, or receives `DATABASE_URL`
from the host env file in 6a.

### 6c. Encrypted secrets in git with SOPS + age (if you want them versioned)
If you want the secret material under version control without exposing it, encrypt it. SOPS
with an age key encrypts values in a committed file; only holders of the age private key (which
is NOT committed, see `.gitignore` `*.agekey`) can decrypt. This fits a self-host, no-cloud
posture and gives you versioned, auditable secret changes. Use it only if you actually need
versioned secrets; for a single box, 6a or 6b is less machinery.

---

## 7. Secret scanning (defense in depth)

Mistakes happen, so the repo catches them mechanically:
- CI runs `gitleaks` on every push (`.github/workflows/ci.yml`). A committed `.env` or an
  inline credential fails the build.
- Run it locally before committing: `gitleaks detect --source . --redact` (or `just scan`).
- Optionally wire it as a pre-commit hook so a leak is blocked before it is even committed.

`.gitleaks.toml` allowlists the example env files and the `CHANGE_ME` placeholders so they do
not trip a false positive.

---

## 8. If a secret was ever committed

Removing it from the current tree is not enough: git history, clones, and forks still hold it.
Do both:
1. Rotate the credential immediately. Run `scripts/gen_secrets.sh` to mint a new DB password and
   session secret, then update Postgres (`ALTER USER ... PASSWORD ...`) and any provider keys.
2. Purge it from history if the repo is shared (for example `git filter-repo`), then force-push,
   and have collaborators re-clone. Rotation matters more than the purge: assume the old value is
   compromised the moment it was pushed.

---

## 9. One unavoidable external dependency

Carrier tracking (scope Section 12) is the single place that reaches outside the self-hosted
boundary, because package location lives at the carrier. The provider key (`CARRIER_API_KEY`) is
a secret handled exactly like the others: in `.env` for dev, the host secret store for prod,
never committed. Everything else in the stack runs local with no cloud dependency.
