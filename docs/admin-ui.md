# 管理界面与事件协议

## 前台个人工作台

网站与管理后台使用独立布局和路由：网站首页 `/`，个人工作台 `/account/*`，
管理后台 `/admin/*`。它们共享服务器账号与 CLI 登录协议；普通成员不能访问管理 API。
前台登录默认进入 `/account`，管理员从网站登录也不会自动跳转后台；只有访问后台时的
站内 `redirect=/admin/...` 才会在验证管理员身份后返回对应管理页面。

前台顶部按 itexp 网站的结构排列：左侧 LOGO 与菜单紧邻，右侧为主题图标和登录/注册；
登录后显示头像菜单。主题支持明亮、黑暗和跟随系统，前台独立使用 `ostool.web.theme`
保存偏好，后台仍使用 `ostool.admin.theme`。移动端保留主题按钮并提供折叠菜单。

公开版个人工作台包括总览、基本信息、账号安全、我的租赁、租赁申请和个人访问令牌。
登录后的设备页提供可申请的开发板目录。基本信息可修改显示名称和邮箱，修改邮箱需验证
当前密码。改密验证旧密码后撤销该账号全部 OAuth 登录、个人访问令牌及已授权设备码，
随后回到网站登录页。已有编辑草稿不会被实时推送覆盖。

申请与正式租赁分开存储。成员提交申请后可在待审核状态撤回；管理员通过后台
“租赁管理 → 租赁申请”批准或拒绝，并可填写审核意见。批准在同一个数据库事务内
重新检查账号、板卡和租赁时间冲突，生成正式租赁并记录审批结果；重复或并发批准不会
生成重复租赁。申请本身不占用时段、不上电、不授予板卡使用权。

| 接口 | 用途 |
| --- | --- |
| `GET /api/v1/account` | 当前用户资料、可申请设备目录、自己的租赁、申请和令牌元数据 |
| `GET /api/v1/account/events` | 当前用户实时投影，支持 Bearer 或 EventSource 的 `access_token` |
| `PUT /api/v1/account/profile` | `{email, display_name, current_password?}` |
| `POST /api/v1/account/password` | `{current_password, new_password}` |
| `POST /api/v1/account/rental-requests` | `{board_id, starts_at, ends_at, notes?}` |
| `DELETE /api/v1/account/rental-requests/{id}` | 撤回自己的待审核申请 |
| `GET /api/v1/admin/rental-requests` | 管理员申请列表，通过 `rental_requests` topic 同步 |
| `POST /api/v1/admin/rental-requests/{id}/review` | `{decision: "approve" \| "reject", note?}` |

前台页面统一使用 `api/events.ts` 中的独立 account store，连接个人 SSE；不订阅管理 SSE，
不在提交后全量 GET，也不轮询列表。个人推送始终按认证用户过滤，并移除管理投影中的
用户列表、硬件控制命令、服务器配置和其他人的租赁。令牌只在创建响应中展示一次，
列表仅包含名称和时间等元数据。切换账号会清理旧的投影并重新连接。

本地版继续无认证，并隐藏全部个人工作台功能；个人 API 和申请管理 API 返回 404。
公开版首次升级时通过 SQLx 自动创建 `rental_requests` 表，不改写已有租赁数据。

源码改动后，`start` 不会替换已运行进程，需要按当前版本执行 `restart`。该命令会通过
`cargo run` 重新构建 React/Vite 资源并嵌入 Rust 可执行文件，然后运行新进程。
HTML 响应使用 `Cache-Control: no-store`，带内容哈希的静态资源可长期缓存。
使用 `OSTOOL_SERVER_WEB_DIST_DIR` 提供预构建资源时，其环境变量和目录变化也会触发构建。
开发脚本在系统提供 `setsid` 时启动独立会话，避免调用终端退出时连带结束服务器；
`status` 和 `stop` 仍通过记录的服务器 PID 工作，其他环境保留 `nohup` 兼容方式。

## 操作流程

管理后台位于 `/admin/`。默认 `OSTOOL_EDITION=local`，保留总览、板卡管理和底部系统设置，免登录且不显示用户/租赁管理。公开版显式设置 `OSTOOL_EDITION=public`，必须登录并校验管理员角色。
列表顶部统一左侧操作按钮、右侧搜索和过滤下拉框；系统设置以 Tab 区分网络、TFTP 和部署配置。
右侧工作区填满视口高度，采用固定工具栏、独立滚动内容、固定底部状态栏的三段布局。
全局框架左侧为导航，上方为固定标题栏；左上 Logo 区与标题栏使用相同高度和主题背景。
标题栏右侧依次为消息、主题、账号头像。消息从 SSE 的同步错误、释放失败、电源失败和
隔离配置中生成，不新增轮询接口；未读时显示角标，打开消息或点击“全部已读”清除本次
页面会话内的未读状态，提醒内容变化后重新标记为未读。这里不是持久化站内信或聊天功能。
主题菜单支持明亮、黑暗、跟随系统，选择保存在浏览器并在刷新后恢复；系统模式随设备
偏好变化。公开版头像菜单包含账号、设置、网站和退出登录，本地版只保留工作区相关入口。
列表底栏支持每页 10/20/50/100 条和前后翻页；分页在现有 SSE 数据中完成，不增加 GET 或轮询。
筛选改变后返回第一页，实时推送保留当前页和滚动位置；记录删除后自动收敛到有效页。
开发板编辑和系统配置的保存、取消按钮固定在底栏，表单校验和并发修改提示继续生效。
UI 验证可运行 `pnpm --dir ostool-server/webui test:e2e`。如果 UDP 2998 正被已有
服务使用，设置 `OSTOOL_TEST_DISABLE_DISCOVERY=true` 可测试其余页面流程，此时明确
跳过依赖真实 UDP 发现的用例；不会停止已有服务或改变默认发现测试。
开发板页面用紧凑筛选栏搜索名称、板型、MAC，并按板型、标签、状态筛选；
已配置、发现设备和虚拟设备通过标签页切换。状态与常用编辑操作优先展示。

新建板卡的顺序为：填写电源配置 → 手动上电 → 选择发现的 MAC 或手工输入 → 保存。
上电只要求所选电源模块的必要参数，不要求板卡 ID、MAC、串口或 Session。
选择发现设备不会自动填写板型、电源或串口，保存前不会生成半成品板卡。
QEMU 使用已有虚拟设备，保留原来的真实网络发现与匹配校验。

选择 axloader 后，`BoardEditor` 隐藏手动串口开关、SN/path；可选启用“指定串口参数”并持久化保存，未启用时不配置覆盖，保存 `serial: null`。
指定参数优先于 axloader 上报，未指定时使用上报值或固件默认 115200/8N1。切回 U-Boot 保留编辑中的手动串口草稿。实际参数、端口、错误和 `SerialRuntimeStatus`
由会话的 SSE 展示；`serial_manager` 展示发现请求数与候选监听数。没有黑名单界面。
继电器配置继续保留，server 在发现前排除其所有路径别名和 SN。

HTTP Boot 的 `boot_arch` 保留在“高级启动设置”中；留空沿用现有 CLI 的
`x86_64` 默认值，并不表示自动识别。设备上报架构只读展示，普通绑定无需填写该项。

开关机命令提交后由服务器持有，关闭页面不取消、不自动断电。实体板没有电源
反馈时，只显示命令完成。电源资源正在执行操作或属于使用中/释放中的板卡时，
冲突请求返回 409。Custom 命令只能按相同命令配置识别资源，不能推断不同命令
是否控制同一物理设备。Linux 管理端 Custom 命令限时 60 秒，超时终止并回收该
命令创建的进程组。既有 CLI 电源契约不变。

表单始终保留用户输入；其他客户端改变配置时显示冲突提示，由用户选择载入
服务器版本。列表更新不清空内容，重连不重置筛选与滚动。会话剩余时间是本地
时钟显示，不发起网络请求。强制释放返回后继续显示释放过程，实际完成后才移除。

## HTTP 接口

### 电源操作

`POST /api/v1/admin/power-actions`：

```json
{
  "request_id": "client-generated-unique-id",
  "action": "on",
  "power_management": {
    "kind": "zhongsheng_relay",
    "key": { "kind": "serial_number", "value": "relay-serial" }
  }
}
```

`action` 为 `on` 或 `off`，`power_management` 复用板卡配置的三个变体。
返回 `202`：`{ "id": "...", "action": "on", "state": "running", "message": null }`。
终态为 `succeeded` 或 `failed`，通过 SSE 的 `power_actions` 发布，也可单次查询
`GET /api/v1/admin/power-actions/{id}`。

同一服务器进程内，同一请求 ID 和相同参数返回原操作；改变参数返回 409。
请求 ID 长度为 1–128 字符。当前最多保留 4096 个操作，不丢弃幂等键，新请求超限
返回 503。进程重启后操作记录不恢复，查询返回 404；客户端不能自动重发未知动作。
发生网络错误时 UI 提供查询入口，清除提示不会执行额外电源动作。

### SSE

`GET /api/v1/admin/events` 使用 `text/event-stream`。同一页面只创建一个连接。
服务端保活不包含业务更新；代理应关闭 SSE 缓冲并允许长连接。

```text
id: <epoch>:<revision>
event: snapshot
data: {"epoch":"...","revision":1,"kind":"snapshot","data":{"boards":[],"sessions":[],"loaders":[]}}
```

后续事件名为 `update`，信封字段相同，`data` 仅包含改变的资源集合。
集合键包括 `quarantined_boards`、`boards`、`runtimes`、`sessions`、`loaders`、`ota`（镜像和升级任务）、`virtual`、`dtbs`、
`serial`、`serial_manager`、`network`、`server`、`tftp`、`tftp_status`、`overview`、`power_actions`。
集合更新是该集合的完整替换；消失的实体表示删除。前端按稳定 ID/MAC/name
合并并复用未变化的对象，不重新挂载整张表或表单。

事件投影由单一发布器串行维护。业务模块提交后使相关集合失效，由发布器读取
对应所有者；没有后台循环调用全部 GET。快照和 revision 在同一同步边界取出，
订阅在读取快照前建立，读取期间发生的变化会进入后续更新，不会漏掉。

浏览器重连通过 `Last-Event-ID` 请求重放。服务器保留最近 256 条更新；epoch
不同、游标非法、超过保留窗口或消费者落后时发送新快照。客户端遇到 revision
跳跃会重新订阅获取快照。读取单个资源失败时发送 `{ "error": "..." }`，客户端
保留该资源最后一次成功的数据并展示错误。

Loader 的在线期限为 10 秒、设备记录保留 24 小时，到期会主动发布变化。
Linux 使用 inotify 监听串口设备节点、netlink 监听网卡/IPv4、systemd D-Bus
监听服务状态、SIGCHLD 监听 QEMU 退出；QEMU 串口连接变化单独通知。
这些监听需要新增 `notify`、`netlink-sys`、`zbus` 依赖；命令组清理由 `nix` 提供。
非 Linux 平台仍接收应用内部事件，系统级热插拔监听目前仅在 Linux 启用。

## 开发与验证

前端使用 React、TypeScript、Vite、Tailwind 和官方 shadcn 源码组件，pnpm 固定
为 10.33.0。Prettier 用于维护 TSX/样式的一致格式；测试采用 Vitest、React Testing
Library 和 Playwright。构建继续复制前端到 Cargo 输出目录、冻结锁文件安装、
构建并嵌入资源；不提交依赖目录或构建产物。

```bash
pnpm --dir ostool-server/webui install --frozen-lockfile
pnpm --dir ostool-server/webui test
pnpm --dir ostool-server/webui build
cargo test -p ostool-server
cargo build -p ostool-server
pnpm --dir ostool-server/webui exec playwright install chromium
pnpm --dir ostool-server/webui test:e2e
```

浏览器测试启动临时目录下的真实服务器及 Vite 代理，使用 TCP 4174/4175、UDP
2998，要求端口空闲；不会修改宿主服务或 `/etc/ostool-server`。自定义服务器二进制
使用 `OSTOOL_TEST_SERVER_BIN`，已有 Chromium 可用 `PLAYWRIGHT_CHROMIUM_EXECUTABLE`。
测试覆盖真实电源提交、UDP/HTTP 发现、SSE 更新、双客户端冲突、Loader 无请求
离线、DTB 管理、会话释放、配置保存和移动端布局。

## 本地演示数据

一键开发脚本默认启用 `OSTOOL_DEV_DEMO=true`。已有环境使用 `restart` 即可补充数据：

```bash
# 本地版：不需要登录和 PostgreSQL
ostool-server/scripts/dev-env.sh restart
# 公开测试版：PostgreSQL + 登录 + 用户、角色、租赁
OSTOOL_EDITION=public ostool-server/scripts/dev-env.sh restart
```

按当前版本选择其中一条；默认端口相同，切换版本先以原版本执行 `stop`，
或通过 `OSTOOL_DEV_SERVER_PORT` 给两个版本分配不同端口。

访问 `http://127.0.0.1:2999/` 或 `/admin/overview`。首次填充包括：

- 48 块开发板：六种板型、三种启动方式、不同标签、长文本与空备注，包含空闲、
  使用中、释放中、异常及禁用状态。
- 24 个展示用 DTB、12 个持续保活的会话，以及会话下可下载的文本与日志文件。
- 18 个发现设备：在线、离线、冲突和未绑定设备；顶部消息菜单也包含演示错误通知。
- 公开测试版另有 36 个用户（含禁用账号）、6 个自定义角色、72 条过去、当前、未来和
  已取消的租赁，以及 48 条待审核、已批准、已拒绝和已撤回的申请。
  可检查搜索、过滤、分页及编辑效果。

公开测试版演示管理员 `demo01@example.test`、普通用户 `demo02@example.test`，
密码均为 `demo-user-password-123`。原有默认管理员仍为
`admin@example.com` / `admin-password-123`。角色名称仅用于演示现有权限模型。

数据通过正常数据库/文件存储和 API/SSE 展示，名称使用 `demo` / `[演示]` 标识。
重复启动仅补缺失记录，保留已经修改的账号、密码、板卡、DTB 和租赁。
会话和设备在线状态在进程运行时保活；手动释放的会话本次运行内不会重新创建。
运行状态在重启时重新生成，已保存的租赁时间不自动改期。

服务端 `--dev-demo` 仅接受回环监听及回环登录地址，关闭设备发现监听、QEMU、网络测试、
TFTP 服务启动及启动自动断电。演示板卡电源命令为空操作，设备 IP 使用文档地址段，
DTB 不含真实硬件定义。这里用于页面展示，真实硬件联调需关闭演示模式。

使用 `OSTOOL_DEV_DEMO=false` 加同版 `restart` 可停止填充并恢复正常硬件启动流程，
已经持久化的示例数据仍会保留。`clean` 会删除所选开发版目录内的**全部数据**，
下次启动重新生成初始数据。开发脚本不得指向生产数据库或生产数据目录；正常直接启动
和 systemd 部署不会自动开启演示模式。完整配置说明见服务器 README。

## 不兼容板卡配置的启动隔离

启动逐文件读取板卡 TOML。语法错误、不支持的枚举、缺失必填字段、校验失败、
文件名与 ID 不一致或重复 MAC 的文件移动到
`<board_dir>/quarantine/<UTC时间>-<UUID>/<原文件名>`，原始字节保持不变。
同目录的 `reason.json` 保存来源、备份路径、时间和完整原因；启动日志和开发板
页面也显示隔离记录。备份名称不会覆盖之前的文件，下次启动不再扫描隔离目录。

目录按文件名排序加载；重复 MAC 保留排序最前的有效配置，隔离后续冲突文件。
其余有效板卡继续提供服务。修正备份文件后，管理员可将其放回板卡配置目录并
在维护窗口重启加载；系统不自动猜测旧字段含义或自动恢复隔离文件。

目录读取、文件读取或备份移动失败仍作为存储错误报告，保留源文件，不以丢失
配置为代价继续启动。顶层服务器配置错误不属于板卡逐文件隔离范围。

U-Boot 通过串口启动，无需绑定 MAC。切换到 U-Boot 后隐藏 MAC 选择器，保存时不提交网络身份；切回网络启动时保留尚未保存的 MAC 草稿。

## 公开版租赁管理

租赁是用户对指定开发板在 `[starts_at, ends_at)` 之间的使用权，与现有短期会话租约独立。
同一开发板的非取消租赁不可重叠，相邻时间段允许。时间使用 RFC3339（建议 UTC），UI 按浏览器时区输入和显示。

- `GET /api/v1/admin/rentals`：租赁列表，同步通过 SSE `rentals` topic 推送。
- `POST /api/v1/admin/rentals`：新增，成功返回 201。
- `PUT /api/v1/admin/rentals/{id}`：修改未取消、未结束的租赁时间和备注，成功返回 204。
- `DELETE /api/v1/admin/rentals/{id}`：取消，保留记录，成功返回 204。相关会话通过原有流程终止、断电并清理文件。

新增/编辑请求：

```json
{
  "board_id": "rk3568-01",
  "user_id": "00000000-0000-0000-0000-000000000001",
  "starts_at": "2026-10-21T00:00:00Z",
  "ends_at": "2026-10-21T02:00:00Z",
  "notes": "测试使用"
}
```

时间冲突返回 409，无效时间/用户返回 400，未认证返回 401，非管理员返回 403。
普通用户的板型列表和会话创建按当前有效租赁过滤；到期后心跳不能续长使用权。
管理员保留维护访问。创建租赁不会自动上电；截止时开始正常释放，下一位用户需等待释放完成。
状态包括 `scheduled`、`active`、`expired`、`cancelled`，服务器在时间边界主动推送状态变化。
本地版的认证、用户、角色与租赁 API 返回 404；即使配置 PostgreSQL 也不会自动开启这些功能。
