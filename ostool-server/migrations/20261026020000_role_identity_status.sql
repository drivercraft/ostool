-- 角色编码面向管理和集成，内部 ID 及现有用户关联保持不变。
ALTER TABLE roles ADD COLUMN code TEXT;
UPDATE roles SET code=CASE WHEN builtin THEN id ELSE 'role_' || replace(id,'-','_') END;
ALTER TABLE roles ALTER COLUMN code SET NOT NULL;
ALTER TABLE roles ADD CONSTRAINT roles_code_unique UNIQUE(code);
ALTER TABLE roles ADD CONSTRAINT roles_code_format CHECK(code ~ '^[a-z][a-z0-9_-]{2,63}$');
ALTER TABLE roles ADD COLUMN enabled BOOLEAN NOT NULL DEFAULT true;
ALTER TABLE roles ADD CONSTRAINT builtin_roles_enabled CHECK(NOT builtin OR enabled);

CREATE OR REPLACE FUNCTION apply_role_permissions() RETURNS trigger LANGUAGE plpgsql AS $$
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
