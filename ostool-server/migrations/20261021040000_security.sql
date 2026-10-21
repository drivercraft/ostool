ALTER TABLE users ADD COLUMN security_version BIGINT NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN email_verified_at TIMESTAMPTZ;
ALTER TABLE oauth_sessions ADD COLUMN family_id UUID NOT NULL DEFAULT gen_random_uuid();
ALTER TABLE oauth_sessions ADD COLUMN absolute_expires_at TIMESTAMPTZ;
UPDATE oauth_sessions SET absolute_expires_at=refresh_expires_at;
ALTER TABLE oauth_sessions ALTER COLUMN absolute_expires_at SET NOT NULL;
ALTER TABLE oauth_sessions ADD COLUMN last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE oauth_sessions ADD COLUMN ip TEXT;
ALTER TABLE oauth_sessions ADD COLUMN user_agent TEXT;
ALTER TABLE oauth_sessions ADD COLUMN step_up_until TIMESTAMPTZ;
ALTER TABLE oauth_sessions ADD COLUMN security_version BIGINT NOT NULL DEFAULT 0;
CREATE INDEX oauth_sessions_family_idx ON oauth_sessions(family_id);

CREATE TABLE security_buckets (
    key TEXT PRIMARY KEY,
    count INTEGER NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX security_buckets_expiry ON security_buckets(expires_at);
CREATE TABLE security_challenges (
    id UUID PRIMARY KEY,
    purpose TEXT NOT NULL,
    subject TEXT NOT NULL,
    secret_hash TEXT NOT NULL DEFAULT '',
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    payload JSONB NOT NULL DEFAULT '{}',
    attempts INTEGER NOT NULL DEFAULT 0,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX security_challenges_expiry ON security_challenges(expires_at);
CREATE INDEX security_challenges_subject ON security_challenges(purpose,subject);
CREATE TABLE mfa_credentials (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    secret_ciphertext TEXT NOT NULL,
    last_step BIGINT NOT NULL,
    recovery_hashes TEXT[] NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE FUNCTION invalidate_user_credentials() RETURNS trigger LANGUAGE plpgsql AS $$
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
    END IF;
    IF NEW.email IS DISTINCT FROM OLD.email THEN NEW.email_verified_at := NULL; END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER users_invalidate_credentials BEFORE UPDATE ON users
FOR EACH ROW EXECUTE FUNCTION invalidate_user_credentials();

UPDATE personal_access_tokens SET expires_at=created_at+interval '90 days' WHERE expires_at IS NULL;
ALTER TABLE personal_access_tokens ALTER COLUMN expires_at SET NOT NULL;
