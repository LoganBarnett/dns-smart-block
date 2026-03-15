-- Create users table for tracking who made classification decisions.
-- Supports an implicit admin user (no external_id) for systems without auth,
-- and future auth-system users identified by an external_id (OIDC sub, etc.)
CREATE TABLE users (
  id           SERIAL PRIMARY KEY,
  -- Stable identifier from an external auth system (OIDC sub, OAuth sub, etc.).
  -- NULL for the implicit admin user, which exists outside any auth path.
  external_id  TEXT UNIQUE,
  display_name TEXT NOT NULL,
  created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- The implicit admin user: always present, no external_id.
-- id=1 is relied upon as the default admin identity.
INSERT INTO users (display_name) VALUES ('admin');
