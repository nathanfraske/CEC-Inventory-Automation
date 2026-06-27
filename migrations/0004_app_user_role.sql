-- 0004: operator roles (audit: RBAC). Every account has a role; the default is the
-- least-privileged 'operator'. The first account (created via /auth/bootstrap) is 'admin'.
-- Admin gates the privilege-escalation surface (creating other operators).
ALTER TABLE app_user ADD COLUMN role text NOT NULL DEFAULT 'operator';
