CREATE TABLE device_connections (
    board_id TEXT PRIMARY KEY REFERENCES board_records(id) ON DELETE CASCADE,
    endpoint TEXT NOT NULL,
    ca_pem TEXT,
    token_ciphertext TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE device_file_grants (
    token_hash TEXT PRIMARY KEY,
    file_id UUID NOT NULL REFERENCES session_files(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX device_file_grants_expiry ON device_file_grants(expires_at);

-- Existing installations must not restart an unauthenticated TFTP transport.
UPDATE server_records SET payload=jsonb_set(payload,'{tftp,enabled}','false'::jsonb),updated_at=now() WHERE name='settings' AND payload#>>'{tftp,enabled}'='true';
