-- PostgreSQL owns session artifacts; filesystem copies are disposable transport caches.
CREATE TABLE session_files (
    id UUID PRIMARY KEY,
    session_id TEXT NOT NULL,
    path TEXT NOT NULL,
    size BIGINT NOT NULL CHECK (size >= 0),
    sha256 TEXT NOT NULL,
    uploaded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (session_id, path)
);
CREATE INDEX session_files_session ON session_files (session_id);
CREATE TABLE session_file_chunks (
    file_id UUID NOT NULL REFERENCES session_files(id) ON DELETE CASCADE,
    chunk_index BIGINT NOT NULL CHECK (chunk_index >= 0),
    content BYTEA NOT NULL CHECK (octet_length(content) <= 262144),
    PRIMARY KEY (file_id, chunk_index)
);
