-- Initial PostgreSQL schema shared by local and public editions.
-- This baseline replaces the unreleased development migration chain.
-- Once released, keep this migration immutable and add incremental migrations.
CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- Identity and credentials.
CREATE TABLE roles (
    id TEXT PRIMARY KEY,
    code TEXT NOT NULL,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    administrator BOOLEAN NOT NULL DEFAULT false,
    builtin BOOLEAN NOT NULL DEFAULT false,
    enabled BOOLEAN NOT NULL DEFAULT true,
    revision BIGINT NOT NULL DEFAULT 1,
    permissions TEXT[] NOT NULL DEFAULT ARRAY['board.use','rental.request']::TEXT[],
    allocation_policy JSONB NOT NULL DEFAULT '{"max_duration_minutes":120,"max_concurrent_sessions":1,"min_available_minutes":10,"board_types":[]}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT roles_code_unique UNIQUE(code),
    CONSTRAINT roles_code_format CHECK(code ~ '^[a-z][a-z0-9_-]{2,63}$'),
    CONSTRAINT roles_permissions_limit CHECK(cardinality(permissions)<=64),
    CONSTRAINT builtin_roles_enabled CHECK(NOT builtin OR enabled)
);
INSERT INTO roles (id,code,name,description,administrator,builtin,permissions) VALUES
    ('admin','admin','管理员','管理用户、角色、板卡及系统配置',true,true,ARRAY['system.admin']),
    ('member','member','普通用户','通过 CLI 使用板卡会话，无管理后台权限',false,true,ARRAY['board.use','rental.request']),
    ('advanced','advanced','高级用户','拥有普通用户权限，并可免申请审批使用已开放免申请的设备，无管理后台权限',false,true,ARRAY['board.instant','board.use','rental.request']);

CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username TEXT NOT NULL CONSTRAINT users_username_format CHECK(username ~ '^[a-z0-9][a-z0-9_-]{2,39}$'),
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    display_name TEXT,
    is_admin BOOLEAN NOT NULL DEFAULT false,
    disabled BOOLEAN NOT NULL DEFAULT false,
    role_id TEXT NOT NULL DEFAULT 'member' REFERENCES roles(id),
    security_version BIGINT NOT NULL DEFAULT 0,
    email_verified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    password_changed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_login_at TIMESTAMPTZ,
    last_login_ip TEXT
);
CREATE INDEX users_role_status ON users(role_id,disabled,created_at);
CREATE UNIQUE INDEX users_username_unique ON users(lower(username));

CREATE TABLE oauth_sessions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    client_id TEXT NOT NULL,
    scope TEXT NOT NULL DEFAULT '',
    access_token_hash TEXT NOT NULL UNIQUE,
    refresh_token_hash TEXT NOT NULL UNIQUE,
    access_expires_at TIMESTAMPTZ NOT NULL,
    refresh_expires_at TIMESTAMPTZ NOT NULL,
    family_id UUID NOT NULL DEFAULT gen_random_uuid(),
    absolute_expires_at TIMESTAMPTZ NOT NULL,
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ip TEXT,
    user_agent TEXT,
    step_up_until TIMESTAMPTZ,
    security_version BIGINT NOT NULL DEFAULT 0,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_refreshed_at TIMESTAMPTZ
);
CREATE INDEX oauth_sessions_refresh_idx ON oauth_sessions(refresh_token_hash);
CREATE INDEX oauth_sessions_access_idx ON oauth_sessions(access_token_hash);
CREATE INDEX oauth_sessions_family_idx ON oauth_sessions(family_id);
CREATE INDEX oauth_sessions_user_active ON oauth_sessions(user_id,client_id,absolute_expires_at) WHERE revoked_at IS NULL;

CREATE TABLE oauth_device_codes (
    device_code_hash TEXT PRIMARY KEY,
    user_code TEXT NOT NULL UNIQUE,
    client_id TEXT NOT NULL,
    scope TEXT NOT NULL DEFAULT '',
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    interval_secs INTEGER NOT NULL DEFAULT 5,
    authorized_at TIMESTAMPTZ,
    denied_at TIMESTAMPTZ,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE personal_access_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ
);

-- Inventory and durable server resources. Hardware sessions stay process-local.
CREATE TABLE board_records (
    id TEXT PRIMARY KEY,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    board_type TEXT GENERATED ALWAYS AS (payload->>'board_type') STORED,
    disabled BOOLEAN GENERATED ALWAYS AS (COALESCE((payload->>'disabled')::boolean,false)) STORED,
    tags JSONB GENERATED ALWAYS AS (COALESCE(payload->'tags','[]'::jsonb)) STORED,
    mac_address TEXT GENERATED ALWAYS AS (lower(payload#>>'{network_identity,mac_address}')) STORED,
    boot_kind TEXT GENERATED ALWAYS AS (payload#>>'{boot,kind}') STORED
);
CREATE INDEX board_inventory_type_status ON board_records(board_type,disabled);
CREATE UNIQUE INDEX board_inventory_mac ON board_records(mac_address) WHERE mac_address IS NOT NULL;
CREATE INDEX board_inventory_tags ON board_records USING gin(tags);

CREATE TABLE dtb_records (
    name TEXT PRIMARY KEY,
    disabled BOOLEAN NOT NULL DEFAULT false,
    size BIGINT NOT NULL,
    sha256 TEXT,
    content BYTEA NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
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

-- Scheduled allocations and applications share one ownership authority.
CREATE TABLE board_rentals (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source TEXT NOT NULL DEFAULT 'scheduled' CHECK(source IN ('scheduled','instant')),
    released_at TIMESTAMPTZ,
    board_id TEXT NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    starts_at TIMESTAMPTZ NOT NULL,
    ends_at TIMESTAMPTZ NOT NULL,
    notes TEXT NOT NULL DEFAULT '',
    cancelled_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (starts_at < ends_at)
);
CREATE INDEX board_rentals_schedule ON board_rentals(board_id,starts_at,ends_at) WHERE cancelled_at IS NULL;
CREATE INDEX board_rentals_user ON board_rentals(user_id);
CREATE TABLE session_owners (
    session_id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    rental_id UUID REFERENCES board_rentals(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE rental_requests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    board_id TEXT NOT NULL,
    starts_at TIMESTAMPTZ NOT NULL,
    ends_at TIMESTAMPTZ NOT NULL,
    notes TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','approved','auto_approved','rejected','cancelled')),
    decision_note TEXT NOT NULL DEFAULT '',
    reviewed_by UUID REFERENCES users(id) ON DELETE SET NULL,
    reviewed_at TIMESTAMPTZ,
    rental_id UUID REFERENCES board_rentals(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (starts_at < ends_at)
);
CREATE INDEX rental_requests_user ON rental_requests(user_id,created_at DESC,id DESC);
CREATE INDEX rental_requests_pending ON rental_requests(status,created_at) WHERE status='pending';

-- Security proofs are durable; expiring risk counters live exclusively in Redis.
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
CREATE TABLE password_reset_tokens (
    id UUID PRIMARY KEY,
    purpose TEXT NOT NULL CONSTRAINT password_reset_purpose CHECK(purpose='email:reset'),
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
-- Reset-proof recipient lookup and unconsumed-proof expiry queries.
CREATE INDEX password_reset_tokens_email_hash_purpose_created_at_idx ON password_reset_tokens(email_hash,purpose,created_at DESC);
CREATE INDEX password_reset_tokens_expires_at_idx ON password_reset_tokens(expires_at) WHERE consumed_at IS NULL;
CREATE TABLE password_change_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    security_version BIGINT NOT NULL,
    changed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX password_changes_user_time ON password_change_events(user_id,changed_at DESC);
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

-- Public site configuration and immutable legal history.
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

-- Audit records intentionally retain actor metadata after account removal.
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

-- BEFORE UPDATE triggers execute alphabetically: invalidate first, then metadata.
CREATE FUNCTION invalidate_user_credentials() RETURNS trigger LANGUAGE plpgsql AS $$
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
CREATE TRIGGER users_invalidate_credentials BEFORE UPDATE ON users
FOR EACH ROW EXECUTE FUNCTION invalidate_user_credentials();

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

CREATE FUNCTION protect_legal_revision() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='UPDATE' AND NEW.published_by IS NULL
       AND (to_jsonb(NEW)-'published_by')=(to_jsonb(OLD)-'published_by') THEN RETURN NEW; END IF;
    RAISE EXCEPTION 'published legal revisions are immutable';
END;
$$;
CREATE TRIGGER legal_revision_immutable BEFORE UPDATE OR DELETE ON legal_document_versions
FOR EACH ROW EXECUTE FUNCTION protect_legal_revision();

-- 硬件租约历史独立于活动连接；身份和设备名称保留分配时的快照。
-- 不使用级联外键，删除用户、设备或租赁不能抹去使用记录。
CREATE TABLE session_history (
    session_id TEXT PRIMARY KEY,
    board_id TEXT NOT NULL,
    board_type TEXT NOT NULL,
    user_id UUID,
    username TEXT,
    rental_id UUID,
    client_name TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    last_heartbeat_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    stop_requested_at TIMESTAMPTZ,
    ended_at TIMESTAMPTZ,
    stop_reason TEXT,
    end_reason TEXT,
    release_failures INTEGER NOT NULL DEFAULT 0 CHECK (release_failures >= 0),
    last_release_error TEXT,
    CHECK (ended_at IS NULL OR ended_at >= created_at),
    CHECK ((ended_at IS NULL) = (end_reason IS NULL))
);
CREATE INDEX session_history_created ON session_history(created_at DESC,session_id DESC);
CREATE INDEX session_history_board ON session_history(board_id,created_at DESC);
CREATE INDEX session_history_user ON session_history(user_id,created_at DESC);
CREATE INDEX session_history_open ON session_history(session_id) WHERE ended_at IS NULL;

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

-- Public product information is explicitly authored, separate from private board configuration.
CREATE TABLE device_documents (
    board_id TEXT PRIMARY KEY REFERENCES board_records(id) ON UPDATE CASCADE ON DELETE CASCADE,
    revision BIGINT NOT NULL CHECK (revision > 0),
    document JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE device_connections (
    board_id TEXT PRIMARY KEY REFERENCES board_records(id) ON DELETE CASCADE,
    endpoint TEXT NOT NULL,
    ca_pem TEXT,
    token_ciphertext TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE device_file_grants (
    token_hash TEXT PRIMARY KEY,
    file_id UUID NOT NULL REFERENCES session_files(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX device_file_grants_expiry ON device_file_grants(expires_at);


-- 管理列表及个人中心按创建时间分页，主键作为相同时间记录的稳定排序条件。
CREATE INDEX board_rentals_created ON board_rentals(created_at DESC,id DESC);
CREATE INDEX board_rentals_user_created ON board_rentals(user_id,created_at DESC,id DESC);
CREATE INDEX rental_requests_created ON rental_requests(created_at DESC,id DESC);
CREATE INDEX rental_requests_status_created ON rental_requests(status,created_at DESC,id DESC);

CREATE INDEX board_rentals_instant_active ON board_rentals(user_id,ends_at) WHERE source='instant' AND released_at IS NULL AND cancelled_at IS NULL;

CREATE FUNCTION apply_role_permissions() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.code IS DISTINCT FROM OLD.code THEN
        RAISE EXCEPTION 'role code is immutable' USING ERRCODE='23514';
    END IF;
    NEW.revision := OLD.revision + 1;
    IF NEW.permissions IS DISTINCT FROM OLD.permissions
       OR NEW.allocation_policy IS DISTINCT FROM OLD.allocation_policy
       OR NEW.enabled IS DISTINCT FROM OLD.enabled THEN
        NEW.administrator := 'system.admin'=ANY(NEW.permissions);
        UPDATE users SET is_admin=NEW.administrator,security_version=security_version+1 WHERE role_id=OLD.id;
        UPDATE board_rentals SET cancelled_at=now()
        WHERE source='instant' AND released_at IS NULL AND cancelled_at IS NULL AND ends_at>now()
          AND user_id IN (SELECT id FROM users WHERE role_id=OLD.id);
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER roles_permission_changes BEFORE UPDATE ON roles FOR EACH ROW EXECUTE FUNCTION apply_role_permissions();

CREATE FUNCTION invalidate_instant_allocations() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.role_id IS DISTINCT FROM OLD.role_id OR NEW.disabled IS DISTINCT FROM OLD.disabled THEN
  UPDATE board_rentals SET cancelled_at=now() WHERE user_id=NEW.id AND source='instant' AND released_at IS NULL AND cancelled_at IS NULL AND ends_at>now();
 END IF;
 RETURN NEW;
END;
$$;
CREATE TRIGGER users_instant_allocations AFTER UPDATE OF role_id,disabled ON users FOR EACH ROW EXECUTE FUNCTION invalidate_instant_allocations();
