-- 增加全局唯一用户名；保留原有账号、凭据及数据。
ALTER TABLE users ADD COLUMN username TEXT;
-- 优先使用合法且唯一的邮箱前缀；重名或格式不合法的账号使用主键生成。
WITH candidates AS (
    SELECT id, lower(split_part(email, '@', 1)) AS candidate FROM users
), ranked AS (
    SELECT id, candidate, count(*) OVER (PARTITION BY candidate) AS occurrences FROM candidates
)
UPDATE users u SET username = CASE
    WHEN r.candidate ~ '^[a-z0-9][a-z0-9_-]{2,39}$'
         AND r.candidate !~ '^user_[0-9a-f]{32}$' AND r.occurrences = 1
    THEN r.candidate ELSE 'user_' || replace(u.id::text, '-', '') END
FROM ranked r WHERE r.id = u.id;
ALTER TABLE users ALTER COLUMN username SET NOT NULL;
ALTER TABLE users ADD CONSTRAINT users_username_format
    CHECK (username ~ '^[a-z0-9][a-z0-9_-]{2,39}$');
CREATE UNIQUE INDEX users_username_unique ON users(lower(username));

CREATE OR REPLACE FUNCTION invalidate_user_credentials() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.password_hash IS DISTINCT FROM OLD.password_hash
       OR NEW.email IS DISTINCT FROM OLD.email
       OR NEW.username IS DISTINCT FROM OLD.username
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
