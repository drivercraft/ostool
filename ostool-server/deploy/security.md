# 安全配置与验证

默认本地版继续免登录使用：`OSTOOL_EDITION=local` 不创建登录入口，不开放身份、
用户和租赁 API；现有 ostool 使用 `auth_mode=disabled`。两种版本都使用 PostgreSQL，
切换版本无需搬迁业务数据。公网部署必须显式使用 `OSTOOL_EDITION=public`，不能关闭认证。

## 公网默认行为

- 网站注册、网站登录和后台独立登录必须通过图形验证码：PNG 图片、120 秒有效、
  绑定操作与来源 IP/浏览器标识，每次提交即消耗，包括错误答案。没有生产绕过开关。
- 默认不要求邮箱验证，也不需要 SMTP。注册邮箱不等于已经验证邮箱。
- 新建/修改密码至少 15 个字符、最多 1024 字节；拒绝简单重复口令，允许长口令与 Unicode。
  已有密码继续可登录，不会在升级时改写。密码采用 Argon2，并限制并行计算数量。
- 数据库保存限流与验证状态，多进程共享。登录按账户及 IP 限制，默认六次失败后等待
  15 分钟；认证入口另有每 IP 每分钟 90 次总限制。超限返回 429 和 Retry-After。
  图形验证码不是 DDoS 防护，公网入口仍可按部署需求配置连接数/流量限制。
- 浏览器仅使用 HttpOnly、SameSite=Strict、host-only Cookie；HTTPS 使用 Secure 和
  `__Host-` 前缀。前台与后台分别登录退出，普通用户登录后台返回统一凭据错误。
  JavaScript、localStorage、登录 JSON、SSE URL 中不存放访问/刷新令牌。
- 访问令牌默认 15 分钟；刷新令牌轮换，重放旧刷新令牌会撤销整个登录链。
  默认绝对上限 30 天、请求空闲上限一天，每用户每客户端最多十个活跃登录链。
  刷新不延长绝对期限或敏感操作授权期限。
- 修改密码、角色、禁用账号等会撤销其会话、PAT 与待授权设备码。
  PAT 默认 90 天有效，可指定 1–365 天，只允许 CLI 板卡操作，不允许用户/后台管理。
- 后台修改、审计导出、PAT 创建、CLI 授权和账号安全修改需要最近三分钟内验证身份。
  新登录已完成验证；过期后界面要求当前密码，已开双重验证时还需动态码/恢复码。
- 账号安全页可管理验证器、恢复码、当前登录设备和撤销其他设备，数据通过 SSE 更新。
  撤销登录也会使 SSE 关闭；串口连接在五秒内检测凭据失效并走正常释放流程。
- 服务器检查 Cookie 写请求的 Origin，拒绝跨站请求；只信任显式代理地址传入的真实 IP。
  响应包含 CSP、禁止嵌入、禁止 MIME 猜测、禁止私有响应缓存等安全头。
  HTML 每个请求生成独立 nonce；下载内容使用受限 CSP。
- 安全行为纳入审计。日志详情不存密码、验证码、MFA 密钥、恢复码或原始令牌。

## 双重验证

配置一次生成的 32 字节随机密钥（base64），重启服务即可开放 TOTP 设置：

```bash
openssl rand -base64 32
```

将结果写入权限为 600 的部署环境文件的 `OSTOOL_MFA_ENCRYPTION_KEY`。
密钥在服务启动时读取，不要构建进前端，也不要每次启动重新生成。数据库中 TOTP 秘钥
使用 AES-256-GCM 加密并绑定用户，恢复码只存摘要；动态码每 30 秒更新，允许前后一个
时间步，同一个时间步不能重复使用。每次提供八个一次性恢复码，重新生成会废弃旧码。

可选 `OSTOOL_REQUIRE_ADMIN_MFA=true`：未设置验证器的管理员只能访问设置流程，
完成后才开放后台，管理员不能自行关闭。普通成员仍按自己的账号设置使用验证器。
启停双重验证会撤销其他会话和 PAT，保留本次已验证会话。

**数据库备份必须同时保管加密密钥。** 遗失密钥无法解密已有验证器密钥，恢复码登录也会
失败；不要在日常更新中替换该密钥。服务器和用户设备应保持时间同步。
本地开发脚本为公网测试实例自动创建并保留 `.ostool-server-dev/public/mfa.key`，
不会提交到仓库；生产部署应自行管理部署密钥。

## 可选邮箱验证与找回密码

配置 `OSTOOL_SMTP_HOST`、`OSTOOL_SMTP_FROM`，按服务商设置端口、账号、密码及
`OSTOOL_SMTP_TLS=starttls`（默认，常用端口 587）或 `tls`（常用端口 465）。
SMTP TLS 校验使用系统内置根证书集合；生产不允许远程明文 SMTP。
`loopback_plaintext` 仅供回环地址测试/本机邮件代理。

SMTP 可用后，账号安全页开放验证当前邮箱，登录页开放邮件找回密码。
仅当 `OSTOOL_REGISTRATION_EMAIL_VERIFICATION=true` 时，注册才强制邮件验证码。
邮件码十分钟有效，最多五次尝试，成功后失效；同一邮箱一分钟才能再次发送，每小时最多
五次，来源 IP 每小时最多八次。发信前也需要人机验证。密码恢复保留已开通的双重验证，
恢复后仍需动态码或恢复码登录。未配置 SMTP 时不展示这些入口。

## 代理、域名与设备网络

公网必须指定 `OSTOOL_PUBLIC_BASE_URL=https://boards.example.com`。
HTTP 仅允许 localhost/回环测试地址。服务在启动时校验配置，拒绝不完整 Turnstile、
无密钥的管理员强制 MFA、无 SMTP 的强制邮箱验证等配置。
运行时读取环境，无需为更换域名重新构建。完整示例见 [public.env.example](public.env.example)。

Caddy 同机代理使用 `OSTOOL_TRUSTED_PROXY_IPS=127.0.0.1,::1`，Rust 监听回环；
不要信任任意客户端的 X-Forwarded-For。可选 `OSTOOL_ADMIN_ALLOWED_NETWORKS`
用逗号分隔 IPv4/IPv6 CIDR，限制后台页面和 API 的来源网络。

`OSTOOL_HARDWARE_ALLOWED_NETWORKS` 控制可匿名访问 loader 与启动/共享文件的板卡网络，
默认仅私有网段/回环；最好收紧为实际设备子网。其他来源的启动文件下载需要有效登录及
对应会话的访问权限。Caddy 示例默认屏蔽所有硬件路径，设备继续使用内网地址。
本地版保持原有板卡工作流。不要把本地免认证实例直接转发到公网。

Turnstile 可替代图片：`OSTOOL_CAPTCHA_PROVIDER=turnstile`，
配置 site key、secret 和显式 hostname 列表；服务器校验服务商验证结果、动作、主机名和
一次性本地挑战。默认图片无需第三方服务。未连通真实 Turnstile 服务时不能用图片测试
结果代替 Turnstile 联调。

## 升级与检查

升级前备份数据库和部署环境文件。新 SQLx 迁移增加安全表、会话期限、凭据撤销触发器，
并给已有永久 PAT 补充创建日起 90 天的有效期；过旧 PAT 因此失效。
浏览器必须重新登录，旧 localStorage 凭据会被清理；CLI 的 OAuth Device、刷新、撤销与
Bearer/PAT 协议继续兼容，不增加图形验证码到 CLI。

```bash
export OSTOOL_TEST_DATABASE_URL=postgres://ostool:ostool@127.0.0.1:5432/ostool_tests
export RUST_TEST_THREADS=4
cargo test -p ostool-server
cargo test -p ostool-server --test security_postgres --test auth_postgres \
  --test account_postgres --test audit_postgres -- --ignored
pnpm --dir ostool-server/webui test
pnpm --dir ostool-server/webui build
```

只使用一次性测试数据库。安全测试覆盖真实 PNG、一次性/用途/有效期、CSRF、
前后台隔离、限流、刷新重放、TOTP/恢复码、会话撤销与归属、强制 MFA、可选 SMTP 注册/
恢复和本地匿名接口。SMTP 测试使用回环收件器，不发往公网。

浏览器公网测试额外要求 `OSTOOL_E2E_AUTH_DATABASE_URL` 和
`OSTOOL_E2E_AUTH_DATABASE_SCHEMA=ostool_e2e_...`，并且服务端使用相同测试 schema，
所有地址必须是回环地址。测试直接修改隔离数据库的挑战摘要来提交已知验证码；
应用本身没有测试验证码、返回答案或关闭验证的接口。

依赖说明：PNG 验证码使用 captcha，TOTP/AES-GCM 使用 ring，常量时间比较使用 subtle，
IP 网段使用 ipnet，验证器二维码使用 qrcode，SMTP 使用 lettre。浏览器 pg 仅是测试依赖。
设计依据包括 [OWASP 认证建议](https://cheatsheetseries.owasp.org/cheatsheets/Authentication_Cheat_Sheet.html)、
[OWASP 会话建议](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)
及 [RFC 6238](https://www.rfc-editor.org/rfc/rfc6238)。
