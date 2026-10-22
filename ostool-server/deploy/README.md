# PostgreSQL、运行时配置与公网部署

使用 `cargo build --release -p ostool-server` 构建。构建脚本会嵌入 React 前端资源；
域名、数据库连接地址和密码不应放入 `VITE_*` 变量。同一个二进制文件可部署到不同域名。

二进制文件分别嵌入前台网站和管理后台入口。前台登录地址为 `/login`，管理员登录地址
为 `/admin/login`。两者共用 PostgreSQL 账号，但浏览器会话独立，一方登录或退出不会
影响另一方。反向代理必须同时转发 `/assets/*`、`/admin/*` 和前台路由；Caddy 示例
已经覆盖这些路径。自定义预构建前端目录必须包含 `index.html`、`admin/index.html`
和 `assets/`。

两个版本都要求 PostgreSQL，并共用数据库结构。公网版额外要求 Redis，用于临时安全
计数；本地版不连接 Redis。默认本地版不启用身份认证、用户管理和租赁管理。公网部署
必须显式设置 `OSTOOL_EDITION=public`，服务端会强制身份认证并要求 PostgreSQL。
公网版拒绝 `OSTOOL_AUTH_ENABLED=false`。升级先前已启用认证的部署时，请先补充
版本环境变量。

将[公网环境变量示例](public.env.example)复制为权限受限的文件，填写数据库地址、
域名和初始管理员凭据，然后运行：

```bash
chmod 600 /path/to/ostool.env
./target/release/ostool-server --config /path/to/config.toml --env-file /path/to/ostool.env
```

不指定 `--env-file` 时，服务器读取工作目录下已有的 `.env`。进程环境变量优先于
`.env`，部署环境变量优先于 TOML。布尔值、端口、连接池大小或公网地址非法时，服务
启动失败。数据库地址和初始管理员密码不会写回 TOML，也不会返回给浏览器。初始化
仅创建新的管理员，不会提升已有注册账号的权限或重置其密码。首次初始化后请移除
初始管理员密码。

使用 systemd 时，可通过以下覆盖配置为现有服务指定环境文件（仅为文档示例，不会
安装或修改服务单元）：

```ini
[Service]
EnvironmentFile=/path/to/ostool.env
```

环境文件使用示例中的 `KEY=value` 格式，不使用 shell 的 `export` 或命令替换。
部署监听地址、数据目录和硬件路径保留在 TOML/`.env` 中。新数据库首次从 TOML 初始化
网络、上传和 TFTP 设置，此后通过界面和数据库管理。可选的 `OSTOOL_DATABASE_SCHEMA`
指定数据库模式名称（1–63 个 ASCII 字母、数字或下划线）；启用迁移时会创建对应模式。
切换版本时使用相同的数据库地址和模式即可保留数据。参见
[systemd 环境文件说明](https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html#EnvironmentFile=)。

让 [Caddy](Caddyfile.example) 监听 80/443 端口，Rust 服务监听 `127.0.0.1:2999`。
为 HTTPS 站点配置 DNS 和 `OSTOOL_PUBLIC_BASE_URL`。Caddy 负责 TLS、WebSocket
连接升级和 SSE 流式传输，无需单独运行前端服务；参见
[反向代理文档](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy)。
PostgreSQL、TFTP、UDP 发现、启动文件下载和网络测试监听器应仅在内部网络开放。
硬件使用局域网入口，用户和 CLI 使用公网 HTTPS 入口。Caddy 示例会屏蔽公网硬件路径，
并不会为硬件协议增加公网身份认证。

公网注册和登录默认要求一次性图形验证码。邮箱验证可选，默认关闭；配置 SMTP 后可
使用邮箱验证和密码找回。密码使用 Argon2，新建或修改密码时至少需要 15 个字符。
注册失败统一返回通用 HTTP 400 响应。新账号分配普通成员角色，不能进入管理后台。
浏览器凭据使用相互隔离的 HttpOnly Cookie；CLI 的 OAuth 和个人访问令牌（PAT）
继续使用 Bearer 令牌。TOTP、恢复码、近期身份复核、会话撤销、CSRF、网络限制和
限流说明见[安全配置与升级说明](security.md)。管理员通过 `/admin/users` 和
`/admin/users/roles` 管理用户与角色。目前角色授予板卡访问或完整管理权限，尚未细分到
单块板卡或单项操作。禁用、删除用户或修改其角色、密码时会撤销凭据。内置角色不能
删除或修改权限级别。正在使用的自定义角色必须先解除用户关联，才能修改权限级别或删除。

品牌、联系方式、协议版本、同意记录和账号安全元数据见[站点配置与身份数据说明](site-and-data.md)。

## 数据持久化情况

两个版本均使用 PostgreSQL 保存持久业务数据。部署初始化配置和硬件资源仍保留在
数据库外；实时硬件状态与会话文件有独立的短期生命周期。请备份数据库和部署配置。
运行时业务操作不读取旧文件。

| 数据 | 当前存储来源 | 重启后的行为 |
| --- | --- | --- |
| 用户、角色、OAuth 会话、设备授权码、PAT 摘要（仅公网版） | PostgreSQL | 持久保留 |
| 开发板预约租赁（仅公网版） | PostgreSQL | 持久保留；启动时重新计算使用资格 |
| 开发板清单 | 两个版本均使用 PostgreSQL | 仅使用数据库；已有 TOML 可手动离线导入 |
| DTB 内容与元数据 | PostgreSQL（`dtb_records.content` 为 BYTEA） | 持久保留；不再使用源文件 |
| OTA 镜像、任务与设备观测记录 | PostgreSQL（`ota_images`、`server_records`） | 持久保留；不导入旧 OTA 文件 |
| 可编辑的 TFTP、网络与上传设置 | PostgreSQL（`server_records`） | 新数据库从 TOML 初始化默认值；界面修改在重启后保留 |
| 初始监听地址、数据库地址与模式、版本、密钥和硬件路径 | TOML 与环境变量 | 启动时生效；不复制到数据库设置 |
| 活动租约、串口连接、固件加载器发现、电源操作状态 | 内存 | 重启后不恢复实时硬件会话 |
| 会话文件 | 按租约隔离的文件 | 随租约生命周期清理 |

服务器不提供板卡或 DTB 目录配置、启动文件导入、隔离搬运流程或文件存储回退。
重启后不会将活动会话恢复为硬件连接，也不提供持久化的历史会话租约报表。

## 已有开发板迁移

部署新的二进制文件后，手动运行独立迁移工具。安装和更新脚本只安装该工具，不会
执行迁移。从源码构建时，使用 `cargo build --release -p ostool-server` 构建两个
二进制文件，再通过 `scripts/migrate-boards.sh` 启动迁移。

先备份 PostgreSQL 和已有板卡、DTB 目录，并在正式导入前停止旧服务。在部署环境文件
中设置**目标** `OSTOOL_DATABASE_URL`，按需设置 `OSTOOL_DATABASE_SCHEMA`。
工具仅从 `--config`、`--env-file` 和进程环境变量读取数据库配置，不初始化认证、网络、
电源、串口或硬件服务。进程环境变量优先于环境文件。

如果目标数据库模式已由新版本初始化，可先检查迁移，不修改数据库结构或业务记录：

```bash
ostool-server/scripts/migrate-boards.sh \
  --env-file /path/to/ostool.env \
  --board-dir /path/to/existing/boards \
  --dtb-dir /path/to/existing/dtbs \
  --dry-run
```

移除 `--dry-run` 后正式导入：

```bash
ostool-server/scripts/migrate-boards.sh \
  --env-file /path/to/ostool.env \
  --board-dir /path/to/existing/boards \
  --dtb-dir /path/to/existing/dtbs
```

没有源码目录时，可用相同参数调用已安装的 `ostool-server-migrate-boards`。
可选参数 `--config /path/to/config.toml` 可提供目标数据库配置。正式导入先执行 SQLx
数据库迁移，因此空目标数据库无需先启动硬件服务。`--dry-run` 要求数据库模式已经
初始化，且不会执行数据库迁移。没有板卡引用 DTB 时，可省略 `--dtb-dir`。

工具按名称排序读取指定目录中的直接子文件 `<board-id>.toml`，校验当前板卡配置，
仅导入这些板卡引用的 DTB。不转换更早的板卡格式，也不导入 OTA、设置或账号文件。
写入前检查全部输入以及 ID、MAC、DTB 冲突。新板卡、引用的 DTB 和导入审计事件在
同一事务中提交，任一步失败均整体回滚。已有且内容相同的记录会跳过，可安全重复运行。
已有 ID 或 DTB 名称对应不同内容、MAC 重复、板卡无效或引用的 DTB 缺失时，整体导入
失败，不会静默覆盖或丢弃设备。源文件不会被移动、改写或删除。

成功后启动或重启新服务器以加载导入的设备。按需保留源文件备份；此后修改这些文件
不会生效。后续所有板卡与 DTB 操作均通过管理界面、API 和 PostgreSQL 完成。

## 验证

```bash
# 使用一次性测试数据库；测试会创建隔离的数据库模式。
export OSTOOL_TEST_DATABASE_URL=postgres://ostool:ostool@127.0.0.1:5432/ostool_tests
export RUST_TEST_THREADS=4
export OSTOOL_TEST_REDIS_URL=redis://127.0.0.1:6379/0
cargo test -p ostool-server
pnpm --dir ostool-server/webui test
pnpm --dir ostool-server/webui build
```

PostgreSQL 集成测试使用一次性数据库和隔离的数据库模式：

```bash
cargo test -p ostool-server --test auth_postgres --test account_postgres \
  --test unified_persistence --test audit_postgres --test board_migration --test security_postgres -- --ignored
```

浏览器测试要求明确指定一次性、仅回环访问的 PostgreSQL 数据库。以下命令会启动
隔离的本地版和公网版测试服务，运行两组浏览器测试，并自动停止测试服务
（需确保 4174、4175、4191 和 UDP 2998 端口可用）：

```bash
cargo build -p ostool-server
OSTOOL_E2E_PUBLIC=true \
OSTOOL_E2E_REDIS_URL=redis://127.0.0.1:6379/0 \
OSTOOL_E2E_LOCAL_DATABASE_URL=postgres://ostool:ostool@127.0.0.1:5432/ostool_tests \
  pnpm --dir ostool-server/webui test:e2e
```

每个测试服务创建新的 `ostool_e2e_...` 数据库模式。公网测试在隔离测试数据库中创建
用户和角色、启用双重验证，并直接替换验证码摘要。应用中没有绕过验证码的接口。
未设置 `OSTOOL_E2E_PUBLIC=true` 且未显式配置独立公网测试服务时，会跳过公网账号
测试。外部启动的公网测试服务必须提供配套的 `OSTOOL_E2E_AUTH_BASE_URL`、
`OSTOOL_E2E_AUTH_DATABASE_URL`、`OSTOOL_E2E_AUTH_DATABASE_SCHEMA`、
`OSTOOL_E2E_ADMIN_EMAIL` 和 `OSTOOL_E2E_ADMIN_PASSWORD`；仅接受回环主机和
明确使用 `ostool_e2e_` 前缀的数据库模式。不要使用开发或生产设备数据。
参见[安全测试说明](security.md#升级与检查)。

## 开发板预约租赁

`/admin/rentals` 为已启用的用户分配指定开发板，开发板也须处于启用状态。
使用 UTC 时间区间 `[starts_at, ends_at)`，同一开发板的租赁时段不能重叠，相邻时段
可以衔接。服务器串行处理会话准入和变更，并通过每块板卡的 PostgreSQL 事务级咨询锁
检查时段冲突。

普通成员仅能看到当前已分配的板型，只能在已分配的板卡上建立会话。已有短期会话仍需
正常发送心跳。会话截止时间不超过租赁结束时间，串口活动触发的心跳同样受限。
到期或取消时，现有释放协调器关闭串口流、断电并清理会话文件。实际硬件释放可能晚于
预定截止时间；下一次分配不会绕过板卡正在释放或出错的状态。管理员可不经过租赁，
获取空闲板卡进行维护。

新增租赁不会为硬件上电。编辑租赁时保留原用户和板卡；需要更换时，取消后重新创建。
已取消或已到期的记录仍可查看。板卡存在未完成租赁时，须先取消租赁才能重命名或删除
板卡。本地版不开放这些 API，保持现有 `ostool` 会话行为。
