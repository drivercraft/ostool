# 公网 CLI 登录与租赁鉴权审查

审查日期：2026-10-21。结论：在公网模式和正确的 CLI 配置下，已验证登录、保存凭据、自动刷新、租赁审批、板卡会话及后续 HTTP/WebSocket 操作。申请本身不授予访问权，必须审批通过且进入有效时段。

这份记录覆盖源码与隔离测试环境，不能作为公网实例、TLS 配置和真实开发板已经完成验收的证明。

## 使用前提

租赁、角色、验证码与会话权限由服务端负责，不以同步升级 CLI 为前提。客户端仍沿用原有 OAuth Device、Bearer、Board REST 和 WebSocket 协议；本分支保留的客户端改动仅修复已确认的客户端缺陷，可独立评估升级。服务器使用 `OSTOOL_EDITION=public`、PostgreSQL、Redis 和实际 HTTPS 公网域名。CLI 全局或项目配置明确指定：

```toml
[board]
server = "https://boards.example.com"
auth_mode = "required"
```

```bash
ostool login
ostool board ls
ostool board connect --board-type TYPE --board-id BOARD_ID
ostool board run
ostool logout
```

`login --server` 只覆盖本次登录，不修改全局服务器地址或认证模式。本地版继续使用 `auth_mode = "disabled"`，不启用登录与租赁。

## 客户端改动与兼容性

优先修改 `ostool-server`，不为租赁或认证新增客户端协议、配置字段、验证码参数或登录流程。与原有客户端相比，OAuth URL/表单、JSON 请求与响应解析、Bearer Header、CLI 参数、配置和凭据格式均保留。

| 保留的客户端修复 | 已确认的缺陷 | 对协议的影响 |
| --- | --- | --- |
| 将密钥环访问移出异步执行线程 | Linux 同步 Secret Service 创建嵌套 Tokio runtime 导致崩溃 | 无 |
| 凭据文件私有创建、原子替换、加锁 | 权限在写入后才设置，且并发覆盖可能损坏或丢失凭据 | 无，沿用原文件路径和 JSON 格式 |
| 刷新时在锁内读取最新记录 | 使用另一个进程已轮换的旧刷新令牌会触发重放撤销 | 无，沿用原刷新请求 |
| 401 清理按实际被拒绝的凭据比较 | 旧请求可能误删新的已保存登录，环境变量凭据可能误删独立登录 | 无额外请求，保留原始 401 |
| 统一解析后校验 WebSocket 同源、隐藏 Header 调试值 | 协议相对 URL 绕过同源检查；调试输出可能泄漏 Bearer | 正常同源地址不变，拒绝越界地址 |

已移除上一轮加入的“401 后自动换 token 重放请求”。未知公网后端可能在执行部分业务后返回 401，客户端不能假定可以安全重放 POST/PUT。原有公开的 `invalidate_after_unauthorized()` 方法签名也已恢复。

服务端租赁分配沿用既有繁忙/未找到错误的 `code/message`，使原客户端的设备等待规则继续生效，而不是要求修改客户端错误分类逻辑。

`cli_gateway_contract` 使用基线 `0331dfa` 对应的 HTTP 约定模拟既有网关，检查域名根目录和 `/webapp/ostoolmanagephp/` 前缀两种部署下的原路径、请求体、Bearer、相对串口地址、旧响应缺少扩展字段时的解析，以及 401 不自动重放。这属于协议回归，不等于已对未提供的真实旧公网服务端完成联调。

## 已验证的权限边界

| 操作 | 公网检查 |
| --- | --- |
| Device 登录 | 浏览器密码、验证码、已启用的 MFA、终端码确认；授权码限时且只能兑换一次 |
| 自动刷新 | 刷新令牌轮换、重放检测、登录族撤销、跨进程锁；读取最新持久化凭据 |
| HTTP 收到 401 | 原样返回错误，不自动重放业务请求；不会回退匿名或误删另一个命令保存的新凭据 |
| 查询板型、建立会话 | 有效 token、`board:operate`、当前用户的有效租赁、板卡可用状态 |
| 心跳、启动配置、串口状态、DTB、文件上传下载、电源操作 | 有效 token、当前会话归属及其正式租赁；其他用户返回 404 |
| 串口 WebSocket | 解析全部 URL 后校验同源及协议、握手鉴权、租赁期限；每 5 秒复核当前 access grant 或 PAT |
| 租赁取消、到期、缩短 | 停止会话、关闭串口并释放板卡，心跳不能延长正式租赁 |
| 账号禁用、改密、凭据撤销 | 拒绝新请求并关闭失效凭据对应的现有串口 |
| 账号和后台 API | CLI 不能使用板卡 scope 读取/修改账号或管理后台；后台要求独立管理员登录 |
| 会话详情 | 仅返回设备摘要，不返回电源命令、串口配置和内部备注 |
| 公开设备目录 | 免登录；只返回 ID、板型、标签、禁用状态 |

管理员拥有完整板卡操作权限，普通用户受租赁限制。

## 本次修复

- WebSocket 的 `//host/path` 等协议相对地址可能绕过只针对绝对 URL 的同源检查：改为统一解析后检查，防止 Bearer 被发送到其他站点；凭据 Header 标为敏感，避免调试输出泄漏。
- 会话详情返回完整硬件配置，可能泄漏电源控制命令中的管理凭据：改为字段白名单。
- CLI token 未限制账号接口，`offline_access` 可能被当作板卡操作权：增加客户端与 scope 限制。
- 已建立串口只检查登录族的长期期限，未检查当前 access grant 是否仍有效：到期未刷新也须断开。
- 另一个客户端已经轮换刷新令牌时，仍可能使用缓存中的旧令牌：加锁后读取最新记录，并协调登录、退出和导入。
- 旧请求收到 401 时可能删除新保存的凭据：按实际被拒绝的 token 比较持久化记录，保留新登录供下次显式调用使用；不自动重放请求。
- 本地凭据文件在写入后才设置权限，且原地覆盖：改为私有临时文件、原子替换及跨进程文件锁。
- 可信代理缺失/非法转发头时可能继承代理私网地址：改为未知来源，设备网授权拒绝通过。
- OTA 镜像下载的公网/设备网规则与 loader 不一致：统一限制为实际设备网。
- 设备码页面增加“本人发起并核对终端验证码”确认，参考 [RFC 8628 §5.4](https://www.rfc-editor.org/rfc/rfc8628#section-5.4)。这能降低误授权风险，不能消除实时钓鱼。

## 部署仍须保证的边界

TFTP、固件 loader、设备侧 HTTP Boot/共享文件不是用户 Bearer API。它们依赖受信任设备网络，必须只向实际板卡子网开放。Caddy 公网示例屏蔽这些路径；Rust 若只监听回环，需另行提供仅设备网可达的入口，并确保板端文件 URL 使用该入口。

能运行任意内核或获得 root 的租户也能从板卡网络发起请求。共享一个可互访的“受信任私网”不能作为多租户安全隔离，应使用 VLAN/防火墙隔离设备、服务器和租户流量。数据库、Redis、原始管理监听器及吞吐测试端口不能暴露到公网。

跨租户的设备状态也需要处理：当前释放会话会断电并清理服务器临时会话文件，不等于擦除板载存储或恢复固件。允许租户写 flash/磁盘时，换租前需有可验证的镜像恢复和数据清理流程。

应用层目前限制单文件大小，尚无按用户或会话累计上传配额。向不受信任用户开放前还应补充累计配额、磁盘保留容量与相应压力验证，避免合法账号耗尽服务资源。

需要按部署环境设置可信代理地址、实际硬件子网、随机管理员密码及稳定的 MFA 加密密钥；只执行登录或启用公网界面不代替这些配置。

## 验证与未验证项

`cli_rental_contract` 调用真实 `TokenManager::login_device`、`AuthClient`、Board HTTP client 和串口 client。它使用回环 HTTP、独立 PostgreSQL/Redis 和 PTY，覆盖设备授权、持久化、自动刷新、两个客户端缓存交错、401 原样返回并保留新登录、审批前拒绝、审批后使用、租赁取消/到期、PAT 撤销以及越权路径。60 秒 access TTL 专门用于暴露刷新边界。

`session_ws_lifecycle` 验证串口正常刷新不中断、撤销和未刷新过期后断开并释放。`security_postgres` 验证验证码、MFA、Cookie/CSRF、权限隔离、反向代理边界等。浏览器测试验证真实登录页面的设备码确认与兑换。

复现时，环境变量必须指向一次性测试实例：

```bash
export OSTOOL_TEST_DATABASE_URL=postgres://ostool:ostool@127.0.0.1:5432/ostool_tests
export OSTOOL_TEST_REDIS_URL=redis://127.0.0.1:6379/0
export RUST_TEST_THREADS=4
cargo test -p ostool-server --test cli_rental_contract --test session_ws_lifecycle \
  --test security_postgres --test auth_postgres --test account_postgres -- --include-ignored
cargo test -p ostool-server --test cli_gateway_contract
cargo test -p ostool --lib auth::credential_store::tests
```

尚未在实际公网域名/Caddy/TLS、真实固件、真实 U-Boot/TFTP 或生产网络上验收，不能宣称“零安全缺陷”。
