-- Ownership metadata persists; hardware leases themselves remain process-local.
CREATE TABLE session_owners (
    session_id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
