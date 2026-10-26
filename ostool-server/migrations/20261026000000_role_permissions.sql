-- 角色能力与即时使用规则；保留已有管理员、用户、租赁和会话历史。
ALTER TABLE roles ADD COLUMN revision BIGINT NOT NULL DEFAULT 1;
ALTER TABLE roles ADD COLUMN permissions TEXT[] NOT NULL DEFAULT ARRAY['board.use','rental.request']::TEXT[];
ALTER TABLE roles ADD COLUMN allocation_policy JSONB NOT NULL DEFAULT '{"max_duration_minutes":120,"max_concurrent_sessions":1,"min_available_minutes":10,"board_types":[]}';
ALTER TABLE roles ADD CONSTRAINT roles_permissions_limit CHECK (cardinality(permissions)<=64);
UPDATE roles SET permissions=ARRAY['system.admin'] WHERE administrator;
ALTER TABLE board_rentals ADD COLUMN source TEXT NOT NULL DEFAULT 'scheduled' CHECK (source IN ('scheduled','instant'));
ALTER TABLE board_rentals ADD COLUMN released_at TIMESTAMPTZ;
CREATE INDEX board_rentals_instant_active ON board_rentals(user_id,ends_at) WHERE source='instant' AND released_at IS NULL AND cancelled_at IS NULL;
-- 修改已分配角色时统一撤销旧凭据；缩减权限立即终止即时使用授权。
CREATE FUNCTION apply_role_permissions() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.revision := OLD.revision + 1;
    IF NEW.permissions IS DISTINCT FROM OLD.permissions OR NEW.allocation_policy IS DISTINCT FROM OLD.allocation_policy THEN
        NEW.administrator := 'system.admin'=ANY(NEW.permissions);
        UPDATE users SET is_admin=NEW.administrator,security_version=security_version+1 WHERE role_id=OLD.id;
        UPDATE board_rentals SET cancelled_at=now() WHERE source='instant' AND released_at IS NULL AND cancelled_at IS NULL AND ends_at>now() AND user_id IN (SELECT id FROM users WHERE role_id=OLD.id);
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
