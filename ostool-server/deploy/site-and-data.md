# 站点配置、协议与身份数据

系统设置按选项卡提供以下配置：

| 配置组 | 内容 |
| --- | --- |
| 站点信息 | 名称、简称、标语、描述、标志和图标路径、版权与代码仓库链接 |
| 联系与备案 | 联系邮箱/电话、支持链接、运营主体、地址、ICP备案与公安备案信息/链接 |
| 协议与隐私 | 标题与纯文本正文，发布版本、摘要与历史发布时间 |
| 维护提示 | 是否显示维护提示和提示正文，不会中断业务或登录请求 |
| 邮件服务（公网） | SMTP 配置状态、最近 100 次投递的用途、收件人、结果及时间 |
| 登录与部署（公网） | Redis/PostgreSQL 状态、验证码、注册邮箱验证、MFA/限流策略与部署参数 |
| 网络与上传、TFTP、数据存储 | 延续现有运行配置 |

公开内容保存到 PostgreSQL，不需要重构建前端。网站品牌、页脚、文档页和前后台登录页面
统一使用公开配置；修改通过 SSE 推送，不轮询、不在保存后重新拉全量列表。
编辑草稿不会被推送覆盖；保存使用配置版本进行并发检查，冲突返回 409。

站点标志和浏览器标签页图标使用同源静态图片路径，如 `/branding/logo.png`，可由部署时的 Caddy
静态资源路由托管。此设置不抓取外部 URL，也不提供任意服务器文件读取。
协议为纯文本，页面保留换行并转义 HTML，不执行脚本或渲染运营者输入的原始 HTML。

SMTP 密码、Redis URL、数据库 URL、MFA 加密密钥、公开域名与安全策略仍在受保护的
`.env`/进程环境中管理；系统设置只显示其可公开的状态，不返回密码或连接凭据。
邮箱验证仍是可选配置，默认关闭；本地版不启用登录、用户或租赁功能，也不连接 Redis。

## 协议版本与注册同意

初始提供可编辑的基础文档，部署运营者应发布适合实际站点的协议和隐私内容。
登录页显示阅读链接，注册页要求明确勾选用户协议与隐私政策。后台登录同样有阅读链接。
发布正文或标题会分别增加对应文档版本，旧文档不可覆盖或删除。
修改站点名称、联系信息不会增加协议版本。

注册 POST 必须包含：

```json
{
  "legal": {
    "accepted": true,
    "terms_version": 1,
    "privacy_version": 1
  }
}
```

两个版本来自 `/api/v1/auth/config` 中的 `site` 或 `/api/v1/site/events` 的公开快照。
服务端把新用户和两条同意记录放在同一个事务中，锁定发布行并检查最新版本。
版本已变更时返回 409 `legal_changed`，不创建用户；页面取消已勾选状态并要求重新阅读。
已存在用户的历史同意记录保留原版本，不会因运营者编辑而被改写。
记录包含接受时间、来源 IP 摘要和浏览器标识摘要，不保存原始请求凭据。

## 表结构与字段用途

| 表 | 关键字段和用途 |
| --- | --- |
| `site_settings` | 公开配置 JSON、修订号、修改时间和修改人 |
| `legal_document_versions` | 类型、版本、标题、正文、SHA-256、发布时间和发布人；数据库约束禁止改写正文 |
| `user_legal_acceptances` | 用户、文档版本、同意时间和来源摘要；外键关联发布版本 |
| `users` | 邮箱、密码摘要、角色、禁用状态、凭据版本、邮箱验证时间，新增修改时间、密码修改时间、最近登录时间/IP |
| `roles` | 名称、说明、权限级别、内置标记，新增创建/修改时间 |
| `board_records` | JSONB 保留结构化板卡配置，新增板型、禁用状态、标签、MAC 和启动方式生成列，以及创建时间 |
| `oauth_sessions` | 用户/客户端/权限、访问/刷新摘要、登录链、访问/绝对期限、最近使用、凭据版本、IP/浏览器、复核期限，新增修改及刷新时间 |
| `personal_access_tokens` | 摘要、名称、期限，新增最近使用时间 |
| `email_verification_tokens` | 注册/邮箱验证用途、收件人摘要、验证码摘要、用户/凭据版本、来源摘要、五次尝试上限、发送/消费/失效/到期时间 |
| `password_reset_tokens` | 独立的恢复证明、尝试及生命周期约束；密码或凭据变化会使旧证明失效 |
| `password_change_events` | 实际密码修改事件、用户、凭据版本和时间，不保存新旧密码或密码摘要 |
| `mail_deliveries` | SMTP 用途、收件邮箱/摘要、关联证明、结果、规范化错误码与完成时间；不保存正文、验证码或验证链接 |
| `security_challenges` | 图形验证码、MFA 登录/绑定挑战，保留用途绑定、时效与一次性验证 |
| Redis | 原子限流、登录锁定和邮件冷却等短期计数，配置 TTL；不作为用户或会话数据源 |

板卡生成列从同一份 JSONB 派生，不能单独写入，不会出现两套可编辑权威数据。
索引支持按板型/状态/MAC/标签检索，非空 MAC 在数据库层唯一。
会话、用户状态、邮件和历史记录也有对应索引及外键。
时间和安全事件从数据创建或变更时开始记录。活动硬件连接不在重启后自动恢复。

初始结构直接创建独立的邮箱验证和密码重置表，不保留旧安全表搬运逻辑。
验证码只存摘要；成功、过期、超限或凭据版本变化后不能重新使用。
密码重置保留 MFA，修改密码会撤销已有会话、PAT 和待授权设备码。

## 数据库初始迁移

`migrations/20261022000000_initial_schema.sql` 是当前未发布功能分支的唯一初始迁移。
它直接定义完整表结构、索引、外键与安全触发器，两个版本共用；不再依次执行开发期间
的加列、回填、删列和邮件证明搬运。SQLx 在空库或新的独立数据库模式中执行一次，后续启动
只校验并跳过已完成的迁移。正式发布后的结构变化应新增迁移，不能重写这个文件。

合并前的开发数据库含有旧 `_sqlx_migrations` 记录，不支持直接升级到这个新基线。
不要删除迁移历史、关闭校验或把新迁移标记为已执行来绕过错误；这可能掩盖不完整结构。
保留数据时先备份，并使用新的数据库或 `OSTOOL_DATABASE_SCHEMA`；旧库保持原样，数据迁移
应另行评估。当前服务启动不包含旧数据库模式兼容或自动重置逻辑。

仅当本地开发数据可丢弃时，才可显式重建对应版本的测试环境：

```bash
# 会清除该版本的开发数据库及演示数据；需要保留数据时不要执行 clean。
OSTOOL_EDITION=public ostool-server/scripts/dev.sh clean
OSTOOL_EDITION=public ostool-server/scripts/dev.sh start
```

本地版去掉 `OSTOOL_EDITION=public`。此次代码整理不会自动清理已运行的开发环境。
线上原有文件板卡仍通过独立的 `scripts/migrate-boards.sh` 导入新数据库，
不涉及 SQLx 开发迁移历史，参见 [部署指南](README.md) 的板卡导入流程。

## API 与验证

- `GET /api/v1/site`：公开配置与协议版本。
- `GET /api/v1/site/events`：仅公开配置的 SSE，不含用户、邮箱投递或后台数据。
- `PUT /api/v1/admin/site-settings`：`revision` 与完整 `settings`，需要管理员及近期身份复核。
- `GET /api/v1/admin/legal-history`：最近 100 个发布版本。
- `GET /api/v1/site/legal/{terms|privacy}/{version}`：指定历史版本 JSON。
- `GET /api/v1/admin/mail-deliveries`：最近 100 次邮件投递，网页使用后台 SSE。

公网新增必填 `OSTOOL_REDIS_URL`。开发环境继续使用同一个启动脚本：

```bash
OSTOOL_EDITION=public ostool-server/scripts/dev.sh start
OSTOOL_EDITION=public ostool-server/scripts/dev.sh stop
# 修改代码后重新构建并重启
OSTOOL_EDITION=public ostool-server/scripts/dev.sh restart
```

本地版使用 `ostool-server/scripts/dev.sh start|stop|restart`，无需 Redis。

检查使用一次性 PostgreSQL 和 Redis，分别设置 `OSTOOL_TEST_DATABASE_URL`、
`OSTOOL_TEST_REDIS_URL`、`RUST_TEST_THREADS=4`，运行 `cargo test -p ostool-server`；
身份集成测试使用 `--test security_postgres --test auth_postgres --test account_postgres
--test audit_postgres --test unified_persistence --test board_migration -- --ignored`。

浏览器测试先 `cargo build -p ostool-server`，然后：

```bash
OSTOOL_E2E_PUBLIC=true \
OSTOOL_E2E_REDIS_URL=redis://127.0.0.1:6379/0 \
OSTOOL_E2E_LOCAL_DATABASE_URL=postgres://ostool:ostool@127.0.0.1:5432/ostool_tests \
  pnpm --dir ostool-server/webui test:e2e
```

Redis 计数参考 [Redis INCR 与限流模式](https://redis.io/docs/latest/commands/incr/)；
Rust 使用 redis-rs 客户端的连接管理、超时与重连。
