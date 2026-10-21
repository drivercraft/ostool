-- Both editions use the same durable schema. Files become import sources or caches.
ALTER TABLE dtb_records ADD COLUMN content BYTEA;
CREATE TABLE server_records (
    name TEXT PRIMARY KEY,
    payload JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE ota_images (
    sha256 TEXT PRIMARY KEY,
    metadata JSONB NOT NULL,
    content BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
