CREATE TABLE audit_events (
    id UUID PRIMARY KEY,
    request_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ,
    actor_id UUID,
    actor_name TEXT NOT NULL,
    actor_kind TEXT NOT NULL,
    source TEXT NOT NULL,
    action TEXT NOT NULL,
    summary TEXT NOT NULL,
    resource TEXT NOT NULL,
    target_id TEXT,
    method TEXT,
    route TEXT,
    ip TEXT,
    user_agent TEXT,
    result TEXT NOT NULL CHECK(result IN ('pending','success','accepted','denied','failed')),
    status_code INTEGER,
    duration_ms BIGINT,
    details JSONB NOT NULL DEFAULT '{}'
);
CREATE INDEX audit_events_created ON audit_events(created_at DESC,id DESC);
CREATE INDEX audit_events_action ON audit_events(action,created_at DESC);
CREATE INDEX audit_events_resource ON audit_events(resource,created_at DESC);
CREATE INDEX audit_events_actor ON audit_events(actor_id,created_at DESC);
CREATE INDEX audit_events_request ON audit_events(request_id);
CREATE INDEX audit_events_result ON audit_events(result,created_at DESC);
