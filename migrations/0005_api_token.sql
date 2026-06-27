-- 0005: service-account API tokens for external/machine-to-machine integration (scope §18/§19).
-- An external app (e.g. the cec.direct build platform) authenticates with a bearer token instead
-- of a cookie session. Only the SHA-256 hash of the token is stored — the plaintext is shown
-- once at creation and never again. Tokens carry a role (operator/admin) like users do, and can
-- be revoked. Admin-only to mint/list/revoke.
CREATE TABLE api_token (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  label        text NOT NULL,
  token_hash   text NOT NULL UNIQUE,           -- hex(sha256(plaintext))
  role         text NOT NULL DEFAULT 'operator',
  created_by   text,
  created_at   timestamptz NOT NULL DEFAULT now(),
  last_used_at timestamptz,
  revoked_at   timestamptz
);

CREATE INDEX api_token_active_idx ON api_token (token_hash) WHERE revoked_at IS NULL;
