# 管理界面与事件协议

## 操作流程

管理后台位于 `/admin/`，保留总览、开发板、DTB、会话租约、TFTP、Server 配置。
开发板页面用紧凑筛选栏搜索名称、板型、MAC，并按板型、标签、状态筛选；
已配置、发现设备和虚拟设备通过标签页切换。状态与常用编辑操作优先展示。

新建板卡的顺序为：填写电源配置 → 手动上电 → 选择发现的 MAC 或手工输入 → 保存。
上电只要求所选电源模块的必要参数，不要求板卡 ID、MAC、串口或 Session。
选择发现设备不会自动填写板型、电源或串口，保存前不会生成半成品板卡。
QEMU 使用已有虚拟设备，保留原来的真实网络发现与匹配校验。

选择 axloader 后，`BoardEditor` 隐藏手动串口开关、SN/path 和线参数，保存 `serial: null`。
切回 U-Boot 保留编辑中的手动串口草稿。实际参数、端口、错误和 `SerialRuntimeStatus`
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
