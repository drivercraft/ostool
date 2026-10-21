-- Durable site configuration, immutable legal revisions and proof of acceptance.
CREATE TABLE site_settings (
    id SMALLINT PRIMARY KEY CHECK(id=1),
    revision BIGINT NOT NULL DEFAULT 1,
    content JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by UUID REFERENCES users(id) ON DELETE SET NULL
);
CREATE TABLE legal_document_versions (
    kind TEXT NOT NULL CHECK(kind IN ('terms','privacy')),
    version BIGINT NOT NULL CHECK(version>0),
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    published_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    published_by UUID REFERENCES users(id) ON DELETE SET NULL,
    PRIMARY KEY(kind,version)
);
CREATE TABLE user_legal_acceptances (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    version BIGINT NOT NULL,
    accepted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ip_hash TEXT NOT NULL,
    user_agent_hash TEXT NOT NULL,
    PRIMARY KEY(user_id,kind,version),
    FOREIGN KEY(kind,version) REFERENCES legal_document_versions(kind,version)
);
ALTER TABLE users ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE users ADD COLUMN password_changed_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE users ADD COLUMN last_login_at TIMESTAMPTZ;
ALTER TABLE users ADD COLUMN last_login_ip TEXT;
ALTER TABLE roles ADD COLUMN created_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE roles ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE board_records ADD COLUMN created_at TIMESTAMPTZ NOT NULL DEFAULT now();
UPDATE board_records SET created_at=updated_at;
-- Searchable relational projections of the canonical board configuration.
ALTER TABLE board_records ADD COLUMN board_type TEXT GENERATED ALWAYS AS (payload->>'board_type') STORED;
ALTER TABLE board_records ADD COLUMN disabled BOOLEAN GENERATED ALWAYS AS (COALESCE((payload->>'disabled')::boolean,false)) STORED;
ALTER TABLE board_records ADD COLUMN tags JSONB GENERATED ALWAYS AS (COALESCE(payload->'tags','[]'::jsonb)) STORED;
ALTER TABLE board_records ADD COLUMN mac_address TEXT GENERATED ALWAYS AS (lower(payload#>>'{network_identity,mac_address}')) STORED;
ALTER TABLE board_records ADD COLUMN boot_kind TEXT GENERATED ALWAYS AS (payload#>>'{boot,kind}') STORED;
CREATE INDEX board_inventory_type_status ON board_records(board_type,disabled);
CREATE UNIQUE INDEX board_inventory_mac ON board_records(mac_address) WHERE mac_address IS NOT NULL;
CREATE INDEX board_inventory_tags ON board_records USING gin(tags);
ALTER TABLE oauth_sessions ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE oauth_sessions ADD COLUMN last_refreshed_at TIMESTAMPTZ;
ALTER TABLE personal_access_tokens ADD COLUMN last_used_at TIMESTAMPTZ;
CREATE INDEX oauth_sessions_user_active ON oauth_sessions(user_id,client_id,absolute_expires_at) WHERE revoked_at IS NULL;
CREATE INDEX users_role_status ON users(role_id,disabled,created_at);
CREATE TABLE password_change_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    security_version BIGINT NOT NULL,
    changed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX password_changes_user_time ON password_change_events(user_id,changed_at DESC);
CREATE FUNCTION record_user_metadata() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at:=now();
    IF NEW.password_hash IS DISTINCT FROM OLD.password_hash THEN
        NEW.password_changed_at:=now();
        INSERT INTO password_change_events(user_id,security_version) VALUES(NEW.id,NEW.security_version);
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER users_metadata BEFORE UPDATE ON users FOR EACH ROW EXECUTE FUNCTION record_user_metadata();
CREATE FUNCTION touch_updated_at() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN NEW.updated_at:=now(); RETURN NEW; END;
$$;
CREATE TRIGGER roles_metadata BEFORE UPDATE ON roles FOR EACH ROW EXECUTE FUNCTION touch_updated_at();
CREATE TRIGGER oauth_sessions_metadata BEFORE UPDATE ON oauth_sessions FOR EACH ROW EXECUTE FUNCTION touch_updated_at();

CREATE TABLE email_verification_tokens (
    id UUID PRIMARY KEY,
    purpose TEXT NOT NULL CHECK(purpose IN ('email:register','email:verify')),
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    email_hash TEXT NOT NULL,
    code_hash TEXT NOT NULL,
    security_version BIGINT,
    request_ip_hash TEXT,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 5),
    expires_at TIMESTAMPTZ NOT NULL,
    sent_at TIMESTAMPTZ,
    consumed_at TIMESTAMPTZ,
    invalidated_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX email_proofs_recipient ON email_verification_tokens(email_hash,purpose,created_at DESC);
CREATE INDEX email_proofs_expiry ON email_verification_tokens(expires_at) WHERE consumed_at IS NULL;
CREATE TABLE password_reset_tokens (LIKE email_verification_tokens INCLUDING DEFAULTS INCLUDING INDEXES);
ALTER TABLE password_reset_tokens ADD CHECK(attempts BETWEEN 0 AND 5);
ALTER TABLE password_reset_tokens ADD CONSTRAINT password_reset_purpose CHECK(purpose='email:reset');
ALTER TABLE password_reset_tokens ADD FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE;
-- Preserve outstanding valid proofs from the previous SQL schema.
INSERT INTO email_verification_tokens(id,purpose,user_id,email_hash,code_hash,security_version,attempts,expires_at,created_at)
SELECT id,purpose,user_id,subject,secret_hash,(payload->>'version')::bigint,LEAST(attempts,5),expires_at,created_at
FROM security_challenges WHERE purpose IN ('email:register','email:verify');
INSERT INTO password_reset_tokens(id,purpose,user_id,email_hash,code_hash,security_version,attempts,expires_at,created_at)
SELECT id,purpose,user_id,subject,secret_hash,(payload->>'version')::bigint,LEAST(attempts,5),expires_at,created_at
FROM security_challenges WHERE purpose='email:reset';
DELETE FROM security_challenges WHERE purpose LIKE 'email:%';
CREATE TABLE mail_deliveries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    challenge_id UUID NOT NULL,
    user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    purpose TEXT NOT NULL,
    recipient_email TEXT NOT NULL,
    recipient_hash TEXT NOT NULL,
    transport TEXT NOT NULL DEFAULT 'smtp',
    status TEXT NOT NULL DEFAULT 'sending' CHECK(status IN ('sending','sent','failed')),
    error_code TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ
);
CREATE INDEX mail_delivery_status_time ON mail_deliveries(status,created_at DESC);
CREATE INDEX mail_delivery_time ON mail_deliveries(created_at DESC,id);
-- PostgreSQL remains the authority; risk counters move to Redis without a fallback.
DROP TABLE security_buckets;

CREATE OR REPLACE FUNCTION invalidate_user_credentials() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.password_hash IS DISTINCT FROM OLD.password_hash
       OR NEW.role_id IS DISTINCT FROM OLD.role_id
       OR NEW.disabled IS DISTINCT FROM OLD.disabled
       OR NEW.security_version IS DISTINCT FROM OLD.security_version THEN
        NEW.security_version := OLD.security_version + 1;
        UPDATE oauth_sessions SET revoked_at=now() WHERE user_id=OLD.id AND revoked_at IS NULL;
        DELETE FROM personal_access_tokens WHERE user_id=OLD.id;
        DELETE FROM oauth_device_codes WHERE user_id=OLD.id;
        DELETE FROM security_challenges WHERE user_id=OLD.id;
        UPDATE email_verification_tokens SET invalidated_at=now() WHERE user_id=OLD.id AND invalidated_at IS NULL;
        UPDATE password_reset_tokens SET invalidated_at=now() WHERE user_id=OLD.id AND invalidated_at IS NULL;
    END IF;
    IF NEW.email IS DISTINCT FROM OLD.email THEN NEW.email_verified_at := NULL; END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION protect_legal_revision() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='UPDATE' AND NEW.published_by IS NULL
       AND (to_jsonb(NEW)-'published_by')=(to_jsonb(OLD)-'published_by') THEN RETURN NEW; END IF;
    RAISE EXCEPTION 'published legal revisions are immutable';
END;
$$;
CREATE TRIGGER legal_revision_immutable BEFORE UPDATE OR DELETE ON legal_document_versions
FOR EACH ROW EXECUTE FUNCTION protect_legal_revision();
