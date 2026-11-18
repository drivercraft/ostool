-- 高级用户仅比普通用户增加免审批使用设备权限，不授予后台管理能力。
-- 保留现有用户的角色和凭据，新注册用户仍默认使用 member。
INSERT INTO roles (id,code,name,description,administrator,builtin,permissions,enabled)
VALUES (
    'advanced',
    'advanced',
    '高级用户',
    '拥有普通用户权限，并可免申请审批使用已开放免申请的设备，无管理后台权限',
    false,
    true,
    ARRAY['board.instant','board.use','rental.request'],
    true
);
