-- Public product information is explicitly authored, separate from private board configuration.
CREATE TABLE device_documents (
    board_id TEXT PRIMARY KEY REFERENCES board_records(id) ON UPDATE CASCADE ON DELETE CASCADE,
    revision BIGINT NOT NULL CHECK (revision > 0),
    document JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
