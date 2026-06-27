-- Operator accounts for the thin app login (scope §18). Argon2 password hashes only — never
-- a plaintext password. Migrations are append-only; this does not edit 0001_init.
CREATE TABLE app_user (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  username      text NOT NULL UNIQUE,
  password_hash text NOT NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
);
