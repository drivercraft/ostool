# axloader 网络控制与本地验证

## 1. 设备所有权

### 1.1 入口与发现

设备控制唯一使用协议 v7。当前 ostool-server 不接收无认证 UDP 发现或旧 poll/status 回调。
管理员先创建开发板并绑定 MAC，再在“设备认证”中配置固定 HTTPS 来源、独立 Bearer 凭据和可选私有 CA。
凭据加密存储在 PostgreSQL，接口不返回 token。控制器只探测已配置的 HTTPS 来源，
核验证书和设备凭据后核对 MAC 与启动代次；未认证报文不能决定回调地址。
设备 `/status` 必须报告协议 v7；boot、serial 和 OTA 写请求携带
`X-Axloader-Protocol-Version: 7`。架构名称是开放字符串，只用于显示，不是准入条件。
设备没有可写 ESP 或 OTA 状态区时，状态响应中的 `ota` 可以为空；服务端不下发
升级镜像，并为任务保存可见的等待原因，不按架构判为失败。

现有 axloader 固件的原生 UDP/HTTP 机制尚未提供这些认证能力，因而不能直接接入这一安全部署。
设备端必须先实现 HTTPS 和 Bearer 校验；服务端没有明文回退或忽略证书的选项。
本文后续设备状态机及手工 QEMU/固件验证描述协议行为，不代表已验证实体设备的 TLS 支持。
U-Boot 则通过已认证用户会话下的服务端 UART/YMODEM 编排使用。

发现页的“创建配置”链接只携带 MAC，`BoardEditor` 不维护或保存启动架构。
发现列表只读显示 axloader 上报的架构；编辑页架构取自当前 `/status` 的 `arch`，在线时只读展示，
离线或未发现时显示未知，不会推定为 x86_64。`arch` 是开放的原始非空字符串，设备新增名称时
server/UI 不要求升级枚举；该值只用于显示和诊断，不参与 MAC、代次或串口绑定身份核对。
设备离线时仍可创建 Session 并上传内核，待上线后由设备判断镜像是否可启动。

HTTP Boot profile 不持久化 `boot_arch`，也不自动迁移旧字段；旧配置由部署时人工修正，
现行解析按未知字段报告错误。`httpboot` 是唯一现行 wire/config kind，`uefi_http` 别名不保留。

### 1.2 启动交接

`BootServer` 在内存维护一个 `DeviceBootJob`。内核和可选 initramfs 由调用方
PUT 上传，各自按清单长度及 SHA-256 检验，每个文件不超过 256 MiB。cmdline
和 initramfs 相互独立且都可省略。CLI/server 上传实际文件字节，不检查 ELF machine、
位宽或本地入口，也不发送架构、镜像格式或入口 Header。共享 artifact 准备仍保留真实容器
坏格式诊断；解析得到 `Architecture::Unknown` 或无架构信息不能作为 HTTP Boot 拒绝理由。
上传校验直接使用本次请求体，在写入内核文件前完成；同一会话的内核写入、命令发布与设备交接串行执行。
交接在等待互斥锁前订阅电源代次及会话停止信号；等待和执行期间优先处理取消，避免旧交接越过开关机操作。
串口交接前继续校验文件长度、摘要、epoch 与绑定；主机不加载或复刻设备 ELF 白名单。
设备在 `start` 中一次性解析真实镜像并应用其本地 machine、ELF 结构、装载范围和 EFI
入口符号规则；拒绝以开放 `{code,message}` 返回并使 boot job 进入 `failed`。
`Failed` 是该会话串口运行时的终态；关闭和迟到的状态更新均保留最初的错误原因。
交接前失败会在服务端保留设备原始 `code/message` loader 状态，并在关闭串口 WebSocket 前发送安全错误提示；
用户 API 隐藏原始错误消息，CLI 保留轮询、WebSocket 关闭及清理期间收到的首个错误，避免会话释放后的 `404` 掩盖它。
设备本地 `TargetSpec` 保存实际 machine 和 EFI 符号名；符号表必须保留，设备只在
`start` 时从目标自身规则解析，不使用 ELF `e_entry` 或 `_start` 代替 EFI ABI 交接。
`POST /api/v1/boot/jobs/{id}/start` 按设备自身的目标契约校验入口，把 cmdline 安装为 EFI LoadOptions，
并仅在归档存在时发布 TGOS `BootPayload` 配置表；回复 `ready_to_handoff` 后释放
TCP4、UDP4 对象和 UART 身份计时器，进入 UEFI 内核交接。
没有 ESP 写入或 ostool-server 时，同样可以从设备 IP 直连完成启动。

```mermaid
sequenceDiagram
    participant L as axloader
    participant S as ostool-server 或直连工具
    S->>L: HTTPS GET /api/v1/status（证书校验与独立 Bearer）
    S->>L: continue（bound 身份匹配或显式 direct）
    S->>L: POST /api/v1/boot/jobs（v7、X-Boot-Epoch）
    S->>L: PUT kernel；可选 PUT initramfs
    S->>L: POST /api/v1/boot/jobs/{id}/start（X-Serial-Binding）
    L-->>S: 202 ready_to_handoff
    L->>L: 释放网络对象并交接内核
```

ostool-server 保留原有 Session、串口 WebSocket、启动清单和板卡租约。会话
上传文件后，`device::push_boot()` 从会话存储读取并重新核对长度与摘要，
再调用设备接口；它不把文件 URL 交给装载器。设备在同一 Session 内复位时，
服务器观察新启动代次并按原 `boot_id` 重新推送。串口在启动前承载本次身份帧，绑定后承载目标系统输出；每次上电都重新验证身份和参数。
若同一启动代次仍保留其他 `boot_id`，服务器在创建返回 `409` 后按状态中的旧 ID
删除该事务，再尝试创建一次；第二次仍冲突则停止本次推送，等待后续已认证的设备探测。

## 2. 设备协议

### 2.1 启动事务

`GET /api/v1/status` 可无版本 Header，用于任何修改前的 v7 检查。所有 boot、serial 和
OTA mutation 携带 `X-Axloader-Protocol-Version: 7`，并继续携带当前 `/api/v1/status`
返回的 `X-Boot-Epoch`；版本头不替代 epoch、串口 binding、OTA update ID/source 等约束。
Header 缺失、非法或不等于 7 时，设备必须在 body 解析、串口接管、文件写入和事务修改前
返回 HTTP `400`，顶层固定为
`{"code":"unsupported_protocol","message":"axloader protocol 7 is required"}`。

`DeviceBootJob` 只有 `boot_id`、`kernel`、可选 `initramfs` 和可选 `cmdline`；内核及
initramfs 各自只有 `size` 与 `sha256`。请求启用未知字段拒绝，携带旧 `arch`、
`image_format` 或 `entry_symbol` 的伪 v7 请求返回 `400 invalid_boot_manifest`。
`BootServer::create()` 只校验非空合法启动 ID、预期长度/摘要、容量和命令行，不接收
`TargetSpec`，因为此时没有镜像字节。相同 ID 和 manifest 的重试返回当前状态且不重置
失败；主机必须先解析这个成功响应中的 `DeviceBootStatus`，若匹配 job 已为 `failed`
则立即重放首个错误，不能因 `kernel_received=false` 再次 PUT。冲突清单返回 `409`。
设备只保留一个启动事务，可在启动前取消。

| 方法与路径 | 结果 |
| --- | --- |
| `GET /api/v1/status` | 返回 v7、MAC、启动代次、开放架构名称、串口参数/绑定、硬件、启动事务及 OTA 状态 |
| `POST /api/v1/serial/continue` | 当前 epoch 的 `serial_id`、`binding_id` 和 `bound/direct`；相同请求幂等 |
| `DELETE /api/v1/serial/bindings/{binding_id}` | 撤销当前匹配绑定，尚在装载器时恢复身份帧 |
| `POST /api/v1/boot/jobs` | 提交 `DeviceBootJob`；创建返回 `201`，幂等重试返回 `200` |
| `GET /api/v1/boot/jobs/{id}` | 查询事务阶段及已接收文件 |
| `PUT /api/v1/boot/jobs/{id}/kernel` | 原始内核，必须带定长和 `X-Image-Sha256` |
| `PUT /api/v1/boot/jobs/{id}/initramfs` | 原始可选归档，采用相同校验规则 |
| `POST /api/v1/boot/jobs/{id}/start` | 当前 epoch/绑定令牌通过、文件齐备且装载成功返回 `202` 并交接 |
| `DELETE /api/v1/boot/jobs/{id}` | 未交接时释放事务和文件 |

请求头限制 4 KiB；仅接受 `Content-Length`，连接空闲 30 秒后取消，短读和
错误摘要按终态保存到 failed job，上传中断按既有非终态规则处理。`ready` 只表示所需文件
已收齐，不表示 ELF 可执行；`failed` 保留 manifest、boot_id、首个 `code/message` 并释放
kernel/initramfs 大缓冲，两个 received 字段均为 false。GET 和同 manifest 重试必须可查询
failed 状态，PUT/start 先重放首个错误，不能借补传清除失败。长时间接收过程中等待路径继续驱动 UDP 广播；单个设备
一次只处理一个 TCP 连接。需要同时上传或查询的客户端应等当前请求完成。

HTTP 错误统一为顶层 `{code,message}`。版本不符为 `400 unsupported_protocol`；manifest
非法为 `400 invalid_boot_manifest`；容量超限为 `413 image_too_large`；长度和摘要不符为
`422 image_size_mismatch`/`image_digest_mismatch`；ELF、machine、EFI 符号和装载失败使用
设备内部固定 code，并在 message 中保留实际 machine、期望 machine、要求符号和底层原因。
主机以 HTTP 类别和事务状态处理，同时原样展示开放 code/message；新增设备错误码不会导致
反序列化失败。明确 `400`/`413`/`422` 是终态，不等待下一次广播后重复上传。

`POST start` 返回 `202` 只表示准备完成、即将交接，不代表内核已运行。202 写失败时不得
交接，保留 ready 事务允许重新 start。start 已发出但连接中断/超时时，主机只 GET 同一
`boot_epoch + boot_id`：只有查询明确为 `ready` 才可用同 ID 重试一次；failed 按终态处理；
其他情况显示“交接结果未确认”并沿用串口观察和会话超时。不得自动 DELETE/重建事务、更换
boot_id 或跨代重新启动。

### 2.2 装载器升级

axloader 的设备端 OTA 实现（如 `ota::OtaContext`）和 A/B 启动器负责镜像校验与
试运行。设备流式写入非活动槽，完成摘要、PE/COFF 格式、`Machine`、EFI Subsystem
及平台可加载性校验后才持久记录待试升级；确定性拒绝不应提交新 trial 或切换活动槽。
待试槽未收到相同升级 ID、相同来源的确认时拒绝内核启动，复位或加载失败由启动器回滚。

| 方法与路径 | 结果 |
| --- | --- |
| `GET /api/v1/ota/status` | 当前槽、摘要、待试 ID 与最近结果 |
| `PUT /api/v1/ota/image` | 原始镜像字节，定长及 `X-Image-Sha256` 必需；设备校验并暂存后返回 `202`，随后进入复位/试运行流程 |
| `POST /api/v1/ota/confirm` | JSON 升级 ID；只有对应待试槽可以持久提交，成功返回 `200` |

服务端把 OTA 镜像视为不透明的字节对象。`OtaStore` 接受非空且不超过
`MAX_IMAGE_BYTES`（32 MiB）的内容，保存标签、长度及 SHA-256，下发前验证存储文件
长度和摘要；不解析 PE/COFF，也不维护 `Machine` 或 CPU 架构白名单。服务端和
`BoardOta` 均不按设备架构阻断升级，离线可以先排队。普通 HTTP Boot 同样不做主机架构
准入；真实镜像检查和本地 EFI 入口选择由 axloader 完成。

直连上传默认生成 ID，且来源为 `direct`。服务端指派携带持久任务 ID，并保留
`X-Boot-Epoch`、`X-Image-Sha256`、`X-Update-Id`、`X-Update-Source: server`
及准确的 `Content-Length`。`OtaStore::decide()` 只在板卡空闲、有实际 OTA 状态且
没有其他待试升级时下发；状态缺失时保留可见的等待原因。新槽广播后，服务端核对
当前板卡绑定、升级 ID、运行摘要与来源，才调用确认接口；确认不受板卡租约限制，
活动 Session 可以在确认解锁后继续启动。`202` 仅表示暂存，只有新槽实际运行、
设备持久确认且激活摘要匹配后，服务器任务才变为 `succeeded`。直连升级不会被服务端自动确认。

`device::push_ota_image()` 读取受限设备响应，依据 HTTP 状态分类，而不按设备错误码
推断支持哪些架构。以下响应规则避免把确定性拒绝当作网络错误重复发送完整镜像。

| 设备响应 | 任务与后续投递 |
| --- | --- |
| `400`、`413`、`422` | 请求不合法、设备限额或镜像摘要/格式/平台校验拒绝；本次任务一次进入 `failed`，不重发相同镜像 |
| `409` | 先刷新实际设备状态/启动代次，至少等待重试间隔后按状态再决定；不消耗传输失败重试次数 |
| `503` | 显示设备当前缺少 OTA 存储/能力的原因；等待 OTA 状态、能力或启动代次变化，不持续重复 PUT |
| 网络中断、超时、其他 `5xx` | 持久记录错误并有限退避，最多三次投递失败；结果不确定时先核对设备是否已接受，最后一次也在核对后才判定失败 |

服务端内部 `Job.error` 至少保留 HTTP 状态及设备 `{code,message}` 响应；不启用旧错误形状解析。
管理 API/SSE 使用安全错误提示，原始细节通过服务器日志排障。
确定性拒绝的响应体读取失败时仍保留 HTTP 状态和读取
错误，不重传。已暂存镜像的升级 ID 与本任务匹配时不再次下发，兼容暂存阶段
`source` 为空的固件；进入试运行后，仍核对来源为 `server` 且运行摘要匹配才确认。重复或旧代次上报不能改写终态或最初的终态失败原因。

服务重启读取独立于 Session 的镜像库和任务。`Job.next_delivery_at` 是最早允许再次
投递的 UTC 时间或 `null`，`Job.waiting_for_device_change` 表示等待 OTA 状态、能力或
代次变化；旧存储缺失字段时分别恢复为 `null` 和 `false`。取消非终态任务不会远程
改写设备状态，已进入待试槽但未确认的设备按自身 A/B 规则回滚。

管理端可删除没有被非终态任务引用的镜像；终态任务保留自身的摘要、版本和长度，
因此删除镜像不破坏历史任务记录。镜像字节、元数据和任务持久化到 PostgreSQL；
镜像上传和删除通过同一门闩串行，元数据在成功持久化后更新内存投影，
避免并发上传、删除或指派留下悬空任务。

## 3. 内建 QEMU 虚拟板

虚拟板默认关闭，当前固定使用 `x86_64 + OVMF + q35 + TCG`。示例服务端配置：

```toml
[virtual_qemu]
enabled = true
qemu_binary = "/usr/bin/qemu-system-x86_64"
ovmf_code = "/usr/share/OVMF/OVMF_CODE_4M.fd"
ovmf_vars = "/usr/share/OVMF/OVMF_VARS_4M.fd"
axloader_efi = "/opt/ostool/BOOTX64.EFI"
runtime_dir = "/var/lib/ostool-server/qemu"
network_namespace = "ostool-qemu"
bridge = "ostool-br0"
tap_pool = ["ostool-tap0", "ostool-tap1"]
memory_mib = 512
cpus = 2
```

相对路径按配置文件目录解析。`virtual-lab` 需要创建 network namespace、veth、
bridge、TAP 和 dnsmasq，应以具备 Linux `CAP_NET_ADMIN` 的身份运行：

```bash
ostool-server --env-file /etc/ostool-server/.env virtual-lab up
ostool-server --env-file /etc/ostool-server/.env virtual-lab status
ostool-server --env-file /etc/ostool-server/.env virtual-lab down
```

默认客户机网段为 `10.77.0.0/24`，服务端地址为 `10.77.0.1`，dnsmasq/bridge
地址为 `10.77.0.254`，DHCP 池为 `10.77.0.100-200`。三个子命令均可重复调用。
当前自带 axloader 固件尚未实现 HTTPS 与凭据校验，QEMU 网络启动联调需先补齐固件能力。
具备认证能力后，先保存板卡 MAC 与 QEMU 电源配置，再在设备认证中配置 HTTPS 来源及凭据；
控制器探测成功后才显示在线。UDP 广播不会注册设备。MAC 和电源配置必须引用同一个虚拟设备；
串口 provider 从电源取得，板卡不保存串口配置：

```toml
[network_identity]
mac_address = "02:aa:bb:cc:dd:ee"

[power_management]
kind = "qemu"
virtual_device_id = "<virtual_device_id>"

[boot]
kind = "httpboot"
```

`VirtualBoardManager` 持有 QEMU 子进程、TAP、独立 OVMF VARS、QMP socket 和
串口 hub。`On`/`Off` 幂等；关闭先发送 QMP `quit`，超时后才终止进程。串口
hub 保留最近 64 KiB 输出并跨 QEMU 重启，新实例广播新的启动代次。

验收顺序如下：

1. 先为 QEMU 固件实现 HTTPS 与独立凭据校验，核实其证书。
2. 保存板卡 ID、类型、QEMU 电源和 MAC，配置设备认证后启动 QEMU，确认已认证探测显示在线。
3. 使用 `ostool` 创建 Session、上传内核及可选 initramfs/cmdline，确认设备 HTTP 交接和内核串口标志。
4. 保持 WebSocket 和 Session，执行虚拟板 `Off -> On`，确认新启动代次重新取得相同启动事务并再次交接。
5. 关闭 WebSocket，确认 Session 经 `releasing` 回到 `idle` 且 QEMU 退出。
6. 新建 Session 再运行一次，全程不修改绑定。

## 4. 兼容与本地联调

### 4.1 兼容边界

协议唯一为 v7，不保留 v2–v6 discovery、poll、status、入口或配置分支。`/api/v1`
是资源 API 路径；设备协议由同一 `DEVICE_PROTOCOL_VERSION` 和请求头控制。旧写请求因
缺版本头直接返回 `400 unsupported_protocol`，旧 URL 自然不存在，不提供升级代理。
UART 身份帧 `AXLOADER-SERIAL/1 ` 是独立帧格式，继续保持 `/1`。

TGOS 与 ostool 共用不兼容的 `httpboot-protocol 0.6.0`、`ostool 0.32.0` 和
`ostool-serial 0.2.0` 系列；锁文件由 Cargo 重新生成，不手工拼接。部署切换应先暂停
新 Session/OTA，完成或取消活动旧任务并确认设备退出 pending/trial，停止旧 server，
人工备份并整理旧配置和 OTA 观测缓存，再安装 v7 axloader、配套 CLI/server，最后重新
发现和验证。新 server 不承担通过旧协议升级旧 loader 的桥接功能。升级前排空 session，
串口定位只在 RAM 保留。

串口 channel、独占租约、原始参数恢复、继电器排除和错误恢复的代码边界见
[串口所有权](axloader-serial-ownership.md)。U-Boot 继续使用手动串口配置与原步骤。

### 4.2 隔离测试

`ostool-server/scripts/test-axloader-local.py` 调用隔离的 `qemu_serial` 集成测试，
运行真实 ostool-server router、HTTP/WebSocket 监听和 OVMF/QEMU。在临时目录
创建服务存储、OVMF VARS 和 ESP；私有 PTY provider 不枚举或打开实体串口。
QEMU UART 字节原样经过 PTY，`hostfwd` 指向真实客户机 TCP4；夹具直接把已核对
的设备公告地址映射到本地端口，不模拟 UART 身份或设备 HTTP。

Session 配置为 `serial: null`，server 必须自动匹配 UART、应用上报参数、确认
并上传真实 ArceOS UEFI ELF。成功条件来自 WebSocket 中的身份帧和内核
`HOST_CMDLINE`、`HOST_INITRAMFS_PASSED`、`Hello, world!`；退出后等待真实租约归还。
默认使用 TGOS axloader QEMU 测试产物，也可用 `--kernel`、`--initramfs` 指定。

```bash
cargo build -p ostool-server
# 使用独立测试数据库，测试自动创建隔离 schema
OSTOOL_TEST_DATABASE_URL=postgres://ostool:ostool@127.0.0.1:54328/ostool_tests \
  python3 ostool-server/scripts/test-axloader-local.py --tgos /path/to/tgoskits-dev
```

TGOS 单仓用 `cargo xtask axloader test qemu --target x86_64-unknown-uefi` 验证
同一真实 FAT 映像上的直连启动、镜像升级、坏摘要、短请求、待试复位和持久确认。
本地测试不调用安装、更新或 systemd 脚本，不连接 runner，也不刷写实体板卡。
`hostfwd` 证明本地真实 HTTP 反向调用；隔离测试不证明实体网络广播与反向路由；后者
属于日后上线前的单独验收。
