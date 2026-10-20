CREATE TABLE roles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    administrator BOOLEAN NOT NULL DEFAULT false,
    builtin BOOLEAN NOT NULL DEFAULT false
);
INSERT INTO roles (id,name,description,administrator,builtin) VALUES
    ('admin','管理员','管理用户、角色、板卡及系统配置',true,true),
    ('member','普通用户','通过 CLI 使用板卡会话，无管理后台权限',false,true);
ALTER TABLE users ADD COLUMN role_id TEXT NOT NULL DEFAULT 'member' REFERENCES roles(id);
UPDATE users SET role_id='admin' WHERE is_admin;

CREATE TABLE storage_imports (
    name TEXT PRIMARY KEY,
    imported_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
