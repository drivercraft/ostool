import { useEffect, useState, type ReactNode } from "react";
import { Link, useSearchParams } from "react-router-dom";
import {
  ArrowLeft,
  ArrowRight,
  BookOpen,
  Check,
  Copy,
  ExternalLink,
} from "lucide-react";
import { useEdition } from "@/edition";
import { Button } from "@/components/ui/button";

const documents = [
  {
    id: "start",
    group: "开始使用",
    title: "快速开始",
    intro: "从连接第一块开发板开始，建立可复用的构建、启动与调试工作流。",
  },
  {
    id: "cli",
    group: "使用指南",
    title: "命令行工具",
    intro: "使用 ostool 在本地开发环境与远程设备之间工作，管理连接和运行配置。",
  },
  {
    id: "devices",
    group: "使用指南",
    title: "设备与使用时段",
    intro: "了解设备目录、使用申请，以及活动硬件会话的生命周期。",
  },
  {
    id: "deployment",
    group: "运行与维护",
    title: "部署指南",
    intro:
      "同一套 release 程序支持本地设备控制与公网协作，按环境配置网络、数据与访问策略。",
  },
  {
    id: "api",
    group: "开发参考",
    title: "API 与实时事件",
    intro:
      "在现有开发流程中集成设备服务，按统一身份和租赁规则访问 REST 与串口接口。",
  },
  {
    id: "help",
    group: "运行与维护",
    title: "常见问题",
    intro: "排查登录、连接和设备使用问题，了解服务重启后的数据与硬件状态。",
  },
] as const;
type DocId = (typeof documents)[number]["id"];
type Section = { id: string; title: string; content: ReactNode };

function CommandBlock({
  children,
  language = "shell",
}: {
  children: string;
  language?: string;
}) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 2000);
    return () => clearTimeout(timer);
  }, [copied]);
  return (
    <div className="docs-code">
      <div>
        <span>{language}</span>
        <Button
          variant="ghost"
          size="icon"
          aria-label={copied ? "已复制" : "复制命令"}
          onClick={() =>
            void navigator.clipboard
              ?.writeText(children)
              .then(() => setCopied(true))
          }
        >
          {copied ? <Check /> : <Copy />}
        </Button>
      </div>
      <pre>
        <code>{children}</code>
      </pre>
    </div>
  );
}
function Steps({ items }: { items: { title: string; text: string }[] }) {
  return (
    <ol className="docs-steps">
      {items.map((item, index) => (
        <li key={item.title}>
          <span>{String(index + 1).padStart(2, "0")}</span>
          <div>
            <h3>{item.title}</h3>
            <p>{item.text}</p>
          </div>
        </li>
      ))}
    </ol>
  );
}
export function Documentation() {
  const [params, setParams] = useSearchParams();
  const publicEdition = useEdition().edition === "public";
  const requested = params.get("page") ?? "start";
  const current =
    documents.find((item) => item.id === requested) ?? documents[0];
  const origin = window.location.origin;
  const [active, setActive] = useState("");
  const connection = `ostool board ls --server ${origin}\nostool board connect --board-type rk3588 --server ${origin}`;
  const content: Record<DocId, Section[]> = {
    start: [
      {
        id: "overview",
        title: "一个工具链，连接完整工作流",
        content: (
          <>
            <p>
              ostool 是面向操作系统与嵌入式研发的工具集。ostool-server
              集中管理开发板、串口、电源和固件资源；浏览器负责资源配置，CLI
              负责衔接日常开发。
            </p>
            <div className="docs-note">
              当前站点运行
              {publicEdition
                ? "公网协作版，设备操作需要登录和有效使用时段。"
                : "本地版，设备操作免登录，适合受信任的本地网络。"}
            </div>
          </>
        ),
      },
      {
        id: "install",
        title: "安装命令行工具",
        content: (
          <>
            <p>在开发工作站安装 ostool，随后确认工具已可用。</p>
            <CommandBlock>
              {"cargo install ostool\nostool --version"}
            </CommandBlock>
          </>
        ),
      },
      {
        id: "connect",
        title: "连接你的设备服务",
        content: (
          <>
            <p>
              {publicEdition
                ? "先在个人工作台申请设备时段，审批通过后，使用浏览器设备授权流程登录。"
                : "本地版直接连接服务地址，无需创建账号。"}
            </p>
            {publicEdition && (
              <CommandBlock>{`ostool login --server ${origin}`}</CommandBlock>
            )}
            <CommandBlock>{connection}</CommandBlock>
          </>
        ),
      },
      {
        id: "next",
        title: "开始构建与验证",
        content: (
          <Steps
            items={[
              {
                title: "配置项目",
                text: "按照项目的构建配置选择架构、内核与启动方式。",
              },
              {
                title: "分配设备",
                text: publicEdition
                  ? "在已获分配的时段内创建会话，避免团队成员之间发生占用冲突。"
                  : "通过板型或设备 ID 选择本地开发板。",
              },
              {
                title: "运行与调试",
                text: "将构建产物投递到设备，连接串口查看系统输出，完成后释放会话。",
              },
            ]}
          />
        ),
      },
    ],
    cli: [
      {
        id: "configuration",
        title: "配置服务地址",
        content: (
          <>
            <p>
              使用 <code>ostool board config</code>{" "}
              配置全局服务地址。公网版需要显式启用认证；仅执行登录命令不会改写全局服务配置。
            </p>
            <CommandBlock language="toml">{`[board]\nserver = "${origin}"\nauth_mode = "${publicEdition ? "required" : "disabled"}"`}</CommandBlock>
          </>
        ),
      },
      {
        id: "authorization",
        title: publicEdition ? "登录与凭据" : "本地直连",
        content: (
          <>
            <p>
              {publicEdition
                ? "CLI 使用设备授权流程，网站与管理后台的浏览器会话保持独立。访问令牌应保存在系统凭据存储中。"
                : "本地版不启用账号和租赁权限。直接指定服务地址即可使用。"}
            </p>
            <CommandBlock>
              {publicEdition
                ? `ostool login --server ${origin}\nostool auth status --server ${origin}`
                : connection}
            </CommandBlock>
          </>
        ),
      },
      {
        id: "commands",
        title: "设备操作",
        content: (
          <>
            <p>
              列出可用板型与设备，或直接选择板型连接串口。项目配置完成后，使用
              board run 连接构建与运行流程。
            </p>
            <CommandBlock>{`${connection}\nostool board run --board-type rk3588 --server ${origin}`}</CommandBlock>
          </>
        ),
      },
    ],
    devices: [
      {
        id: "catalog",
        title: "浏览设备目录",
        content: (
          <>
            <p>
              设备目录对访客开放，包含设备标识、板型、标签和启用状态。目录不公开内部串口路径、电源命令或操作人员备注。
            </p>
            <Link to="/devices" className="docs-inline-link">
              打开设备目录 <ArrowRight size={16} />
            </Link>
          </>
        ),
      },
      {
        id: "schedule",
        title: publicEdition ? "申请使用时段" : "配置本地设备",
        content: (
          <Steps
            items={
              publicEdition
                ? [
                    {
                      title: "选择设备",
                      text: "在设备卡片中进入申请页，日历会显示已占用和不可选择的时间段。",
                    },
                    {
                      title: "提交申请",
                      text: "选择开始与结束时间并填写使用说明，管理员审核后分配租赁。",
                    },
                    {
                      title: "开始使用",
                      text: "在正式租赁的有效时段内使用 CLI 操作设备，时段结束后会话自动释放。",
                    },
                  ]
                : [
                    {
                      title: "添加设备",
                      text: "在管理后台添加设备，设置板型、启动方式、串口和电源。",
                    },
                    {
                      title: "检查配置",
                      text: "确认设备与控制器处于可通信的网络，并使用稳定串口标识。",
                    },
                    {
                      title: "创建会话",
                      text: "从 CLI 连接设备，服务维护短期租约和串口会话。",
                    },
                  ]
            }
          />
        ),
      },
      {
        id: "lifecycle",
        title: "会话与资源释放",
        content: (
          <p>
            {publicEdition
              ? "活动会话持有真实串口、文件和设备占用。正常退出、租约到期或租赁被取消后，服务会执行释放流程。服务器重启需要重新建立硬件连接，数据库中保存的账号与租赁时段不会因此消失。"
              : "活动会话持有真实串口、文件和设备占用。正常退出或短期租约到期后，服务执行资源释放。服务器重启后重新建立硬件连接，已配置的设备与 DTB 保留在数据库中。"}
          </p>
        ),
      },
    ],
    deployment: [
      {
        id: "prerequisites",
        title: "准备运行环境",
        content: (
          <p>
            服务使用 PostgreSQL 保存业务数据，公网版使用 Redis
            处理限流等短期安全状态。Caddy 提供 HTTPS
            入口；设备侧端口和硬件访问应位于受信任的设备网络。
          </p>
        ),
      },
      {
        id: "environment",
        title: publicEdition ? "配置站点与身份" : "配置站点地址",
        content: (
          <>
            <p>
              {publicEdition
                ? "在启动前配置版本、数据库、站点地址及初始管理员。用户名与邮箱分别全局唯一，用户名登录不区分大小写。"
                : "在启动前配置本地版本、数据库连接及站点地址，本地版无需创建登录账号。"}
            </p>
            <CommandBlock language="dotenv">{`OSTOOL_EDITION=${publicEdition ? "public" : "local"}\nOSTOOL_PUBLIC_BASE_URL=${origin}${publicEdition ? "\nOSTOOL_ADMIN_USERNAME=admin\nOSTOOL_ADMIN_EMAIL=admin@example.com" : ""}`}</CommandBlock>
            <p>
              数据库连接、密码和加密密钥从受保护的环境配置读取，不能放入前端构建参数。
            </p>
          </>
        ),
      },
      {
        id: "launch",
        title: "本地验证与正式安装",
        content: (
          <>
            <p>
              本地验证通过 Docker Compose 运行正式 release 程序。部署使用
              deploy.sh docker up 或 deploy.sh systemd up；两者共用一份
              .env，公网版接入现有 Caddy。
            </p>
            <CommandBlock>
              {
                "ostool-server/scripts/dev.sh start\nostool-server/scripts/dev.sh status\nostool-server/scripts/dev.sh cert"
              }
            </CommandBlock>
            <a
              className="docs-inline-link"
              href="https://github.com/drivercraft/ostool/tree/main/ostool-server"
            >
              阅读完整部署说明 <ExternalLink size={15} />
            </a>
          </>
        ),
      },
      {
        id: "maintenance",
        title: "备份与升级",
        content: (
          <p>
            升级前备份数据库、服务配置、文件目录及长期使用的加密密钥。数据库迁移按版本执行，保留已存在的用户和业务记录；活动硬件会话在重启后重新建立。
          </p>
        ),
      },
    ],
    api: [
      {
        id: "authentication",
        title: publicEdition ? "统一身份与权限" : "本地服务接口",
        content: (
          <p>
            {publicEdition
              ? "公网版设备 API 使用 Bearer 身份，并在后续操作中检查会话归属和租赁时段。前台与后台使用独立的 HttpOnly Cookie；普通成员不能通过个人会话访问后台管理接口。"
              : "本地版设备接口免登录，服务维护设备互斥分配与短期租约。使用完整服务地址访问接口，与 CLI 的设备控制流程保持一致。"}
          </p>
        ),
      },
      {
        id: "endpoints",
        title: "核心接口",
        content: (
          <div className="docs-api-list">
            {[
              ["GET", "/api/v1/board-types", "查看板型与设备供给"],
              ["POST", "/api/v1/sessions", "创建设备会话"],
              ["DELETE", "/api/v1/sessions/{id}", "释放设备会话"],
              ["GET", "/api/v1/site/devices", "读取公开设备目录"],
            ].map(([method, path, text]) => (
              <div key={path + method}>
                <span>{method}</span>
                <code>{path}</code>
                <p>{text}</p>
              </div>
            ))}
          </div>
        ),
      },
      {
        id: "events",
        title: "实时更新",
        content: (
          <p>
            页面通过 SSE
            订阅状态，断线时保留当前内容和草稿，重连后同步新快照。服务暂时不可用与身份失效分别处理，避免升级或重启导致重复登录。
          </p>
        ),
      },
      {
        id: "reference",
        title: "接口契约",
        content: (
          <a
            className="docs-inline-link"
            href="https://github.com/drivercraft/ostool/blob/main/docs/api.md"
          >
            完整 REST、串口和设备协议 <ExternalLink size={15} />
          </a>
        ),
      },
    ],
    help: [
      {
        id: "login",
        title: publicEdition ? "重启后需要重新登录吗？" : "服务重启后会怎样？",
        content: (
          <p>
            {publicEdition
              ? "有效登录凭据保存在数据库中，同一访问地址下正常重启不要求重新输入密码。凭据过期、被撤销或账号安全设置改变时，需要重新验证。更换访问域名或 IP 时，浏览器 Cookie 属于不同站点。"
              : "开发板配置、DTB 和可编辑配置保存在 PostgreSQL。活动串口与硬件占用由当前控制进程持有，重启后需要重新连接设备。"}
          </p>
        ),
      },
      {
        id: "https",
        title: "通过 IP 访问时提示证书不受信任",
        content: (
          <p>
            私有 IP 使用 Caddy
            内部证书。导出根证书并在访问设备的系统中信任它；CLI
            同样使用系统信任库。正式域名可由 Caddy 自动管理公网证书。
          </p>
        ),
      },
      {
        id: "allocation",
        title: "可以看到设备，为什么无法使用？",
        content: (
          <p>
            {publicEdition
              ? "目录可见表示该设备已经配置。公网版还需要正式租赁处于有效时间范围；被禁用、正在释放或由其他会话占用的设备不能重复分配。请先在个人工作台核对申请与租赁状态。"
              : "目录可见表示设备已经配置。被禁用、正在释放或由其他会话占用的设备不能重复分配，请在控制台检查启用状态、串口和电源配置。"}
          </p>
        ),
      },
      {
        id: "connectivity",
        title: "实时连接中断怎么办？",
        content: (
          <p>
            先检查网络与服务器运行状态。页面会自动重连，并保留未保存输入。管理员可以在仪表盘查看连接、数据库和后台任务状态；设备串口与电源故障应结合审计记录排查。
          </p>
        ),
      },
    ],
  };
  const sections = content[current.id];
  useEffect(() => {
    setActive(sections[0].id);
    window.scrollTo({ top: 0 });
  }, [current.id]);
  useEffect(() => {
    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries
          .filter((entry) => entry.isIntersecting)
          .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
        if (visible[0]) setActive(visible[0].target.id);
      },
      { rootMargin: "-100px 0px -55% 0px" },
    );
    document
      .querySelectorAll(".docs-article section[id]")
      .forEach((section) => observer.observe(section));
    return () => observer.disconnect();
  }, [current.id]);
  const index = documents.findIndex((item) => item.id === current.id);
  return (
    <main className="public-inner-page docs-page documentation-layout">
      <aside className="docs-sidebar">
        <div className="docs-sidebar-brand">
          <BookOpen />
          <span>开发者文档</span>
        </div>
        <nav aria-label="文档菜单">
          {["开始使用", "使用指南", "运行与维护", "开发参考"].map((group) => (
            <div key={group}>
              <span>{group}</span>
              {documents
                .filter((item) => item.group === group)
                .map((item) => (
                  <button
                    key={item.id}
                    type="button"
                    className={current.id === item.id ? "active" : ""}
                    aria-current={current.id === item.id ? "page" : undefined}
                    onClick={() => setParams({ page: item.id })}
                  >
                    {item.title}
                    <ArrowRight size={14} />
                  </button>
                ))}
            </div>
          ))}
        </nav>
        <a
          className="docs-source-link"
          href="https://github.com/drivercraft/ostool"
        >
          GitHub 源码 <ExternalLink size={14} />
        </a>
      </aside>
      <article className="docs-article">
        <header>
          <div className="docs-breadcrumb">文档 / {current.group}</div>
          <h1>{current.title}</h1>
          <p>{current.intro}</p>
        </header>
        {sections.map((section) => (
          <section key={section.id} id={section.id}>
            <h2>
              {section.title}
              <a href={`#${section.id}`} aria-label={`定位到${section.title}`}>
                #
              </a>
            </h2>
            {section.content}
          </section>
        ))}
        <nav className="docs-page-navigation" aria-label="文档翻页">
          {index > 0 ? (
            <button
              type="button"
              onClick={() => setParams({ page: documents[index - 1].id })}
            >
              <ArrowLeft size={16} />
              <span>
                上一篇<strong>{documents[index - 1].title}</strong>
              </span>
            </button>
          ) : (
            <span />
          )}
          {index < documents.length - 1 && (
            <button
              type="button"
              onClick={() => setParams({ page: documents[index + 1].id })}
            >
              <span>
                下一篇<strong>{documents[index + 1].title}</strong>
              </span>
              <ArrowRight size={16} />
            </button>
          )}
        </nav>
      </article>
      <aside className="docs-toc">
        <span>本页目录</span>
        <nav aria-label="本页目录">
          {sections.map((section) => (
            <a
              key={section.id}
              href={`#${section.id}`}
              className={active === section.id ? "active" : ""}
            >
              {section.title}
            </a>
          ))}
        </nav>
        <p>代码示例使用当前站点地址。</p>
      </aside>
    </main>
  );
}
