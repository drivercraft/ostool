import {
  ArrowDown,
  ArrowRight,
  CalendarClock,
  CheckCircle2,
  CircuitBoard,
  Command,
  FileText,
  Layers,
  Radio,
  ShieldCheck,
  Terminal,
  Zap,
} from "lucide-react";
import { Link } from "react-router-dom";
import { useEdition } from "@/edition";
import { useSiteSession } from "@/apps/web/session";
import { PublicLayout } from "@/apps/web/components/site-layout";
import { BoardVisual } from "@/apps/web/components/board-visual";
import { Button } from "@/components/ui/button";

export default function Home() {
  const edition = useEdition();
  const publicEdition = edition.edition === "public";
  const { user } = useSiteSession();
  const startPath = publicEdition
    ? user
      ? "/account"
      : edition.allow_registration
        ? "/register"
        : "/login"
    : "/admin/overview";
  return (
    <PublicLayout>
      <main className="public-home">
        <section className="public-hero-section">
          <div className="hero-glow hero-glow-one" aria-hidden="true" />
          <div className="hero-copy">
            <div className="hero-kicker">
              <span className="status-pulse" /> OPEN SOURCE · OS DEVELOPMENT
            </div>
            <h1>
              连接每一块开发板，<em>专注每一次创新。</em>
            </h1>
            <p className="hero-subtitle">
              把构建、启动和串口调试连成一条工作流。统一接入真实硬件与
              QEMU，在终端开发，在浏览器管理。
            </p>
            <div className="hero-actions">
              <Button asChild>
                <a href={startPath}>
                  开始使用 <ArrowRight />
                </a>
              </Button>
              <Button asChild variant="outline">
                <Link to="/docs">
                  <FileText />
                  阅读文档
                </Link>
              </Button>
            </div>
            <div className="hero-trust">
              <span>
                <CheckCircle2 />
                开源工具链
              </span>
              <span>
                <ShieldCheck />
                自主部署
              </span>
              <span>
                <Zap />
                实时同步
              </span>
            </div>
          </div>
          <div className="hero-product">
            <BoardVisual />
            <span className="hero-visual-note">构建 · 启动 · 连接</span>
          </div>
          <a className="home-scroll-link" href="#hardware">
            <span>探索完整工作流</span>
            <ArrowDown size={18} />
          </a>
        </section>
        <section
          className="home-feature-section"
          id="hardware"
          aria-labelledby="hardware-title"
        >
          <div className="home-feature-copy">
            <span className="section-label">01 / DEVICE RESOURCES</span>
            <h2 id="hardware-title">
              真实硬件与虚拟设备，
              <br />
              放在同一个工作区。
            </h2>
            <p>
              按设备类型与标签组织开发资源，统一维护接入配置。切换目标硬件时，让熟悉的工作方式继续沿用。
            </p>
            <ul>
              <li>
                <CheckCircle2 />
                实体开发板与 QEMU 统一管理
              </li>
              <li>
                <CheckCircle2 />
                设备类型、标签与运行状态清晰可查
              </li>
              <li>
                <CheckCircle2 />
                按需接入串口、网络启动与电源控制
              </li>
            </ul>
            <Button asChild variant="outline">
              <Link to="/devices">
                浏览设备 <ArrowRight />
              </Link>
            </Button>
          </div>
          <div className="home-resource-visual">
            <div className="home-visual-heading">
              <Layers />
              <span>你的开发资源</span>
            </div>
            {[
              {
                Icon: CircuitBoard,
                title: "实体开发板",
                text: "连接板卡，在真实硬件上验证系统",
              },
              {
                Icon: Command,
                title: "QEMU 虚拟设备",
                text: "沿用相同的配置与开发入口",
              },
              {
                Icon: Radio,
                title: "统一接入",
                text: "设备信息、启动配置与串口会话",
              },
            ].map(({ Icon, title, text }) => (
              <div className="home-resource-row" key={title}>
                <span className="home-feature-icon">
                  <Icon />
                </span>
                <div>
                  <h3>{title}</h3>
                  <p>{text}</p>
                </div>
              </div>
            ))}
            <div className="home-protocols">
              <span>UART</span>
              <span>U-Boot</span>
              <span>HTTP Boot</span>
              <span>QEMU</span>
            </div>
          </div>
        </section>
        <section
          className="home-feature-section home-feature-reverse"
          aria-labelledby="workflow-title"
        >
          <div className="home-feature-copy">
            <span className="section-label">02 / TERMINAL FIRST</span>
            <h2 id="workflow-title">
              留在终端，
              <br />
              完成从启动到调试。
            </h2>
            <p>
              沿用 ostool
              命令连接设备，上传构建产物并进入串口会话。网站负责资源安排，命令行负责把开发工作继续向前推进。
            </p>
            <ul>
              <li>
                <CheckCircle2 />
                构建产物上传与设备启动衔接
              </li>
              <li>
                <CheckCircle2 />
                服务端协调串口、电源及文件投递
              </li>
              <li>
                <CheckCircle2 />
                {publicEdition
                  ? "登录一次，通过 HTTPS 访问授权设备"
                  : "局域网直接连接，免登录使用"}
              </li>
            </ul>
            <Button asChild variant="outline">
              <Link to="/docs?page=cli">
                查看命令行文档 <ArrowRight />
              </Link>
            </Button>
          </div>
          <div className="home-terminal">
            <div className="home-visual-heading">
              <Terminal />
              <span>ostool / terminal</span>
            </div>
            <pre>
              <code>
                {publicEdition ? "$ ostool login\n\n" : ""}$ ostool board ls
                {"\n\n"}$ ostool board connect --board-type rk3588
              </code>
            </pre>
            <div className="home-terminal-footer">
              <span />
              BUILD → BOOT → DEBUG
            </div>
          </div>
        </section>
        {publicEdition && (
          <section
            className="home-feature-section"
            aria-labelledby="booking-title"
          >
            <div className="home-feature-copy">
              <span className="section-label">03 / SHARED HARDWARE</span>
              <h2 id="booking-title">
                共享设备，
                <br />
                各自拥有专注的时间。
              </h2>
              <p>
                先查看可用日历，再申请需要的使用时段。获授权的成员也可以直接领取空闲设备，系统自动记录申请与使用过程。
              </p>
              <ul>
                <li>
                  <CheckCircle2 />
                  日历呈现已有预约，减少时间冲突
                </li>
                <li>
                  <CheckCircle2 />
                  按角色授权即时分配与使用额度
                </li>
                <li>
                  <CheckCircle2 />
                  租约结束后释放设备，留存使用历史
                </li>
              </ul>
              <Button asChild variant="outline">
                <Link
                  to={
                    user
                      ? "/account/requests/new"
                      : "/login?redirect=/account/requests/new"
                  }
                >
                  安排设备使用 <ArrowRight />
                </Link>
              </Button>
            </div>
            <div className="home-booking-visual">
              <div className="home-visual-heading">
                <CalendarClock />
                <span>从预约到释放，有序衔接</span>
              </div>
              <ol>
                {[
                  ["01", "选择设备", "了解板型，查看可用时段"],
                  ["02", "安排使用", "提交申请，或按权限即时分配"],
                  ["03", "连接与开发", "在授权时段内连接 ostool"],
                  ["04", "释放资源", "结束会话，设备交还资源池"],
                ].map(([number, title, text]) => (
                  <li key={number}>
                    <span>{number}</span>
                    <div>
                      <h3>{title}</h3>
                      <p>{text}</p>
                    </div>
                  </li>
                ))}
              </ol>
            </div>
          </section>
        )}
        <section
          className="home-feature-section home-feature-reverse"
          aria-labelledby="deployment-title"
        >
          <div className="home-feature-copy">
            <span className="section-label">
              {publicEdition ? "04" : "03"} / YOUR INFRASTRUCTURE
            </span>
            <h2 id="deployment-title">
              部署在你掌握的环境，
              <br />
              为团队建立清晰边界。
            </h2>
            <p>
              从实验室局域网到公网协作，按使用场景选择访问方式。设备通信留在服务器侧，用户通过统一入口使用获授权的资源。
            </p>
            <ul>
              <li>
                <CheckCircle2 />
                本地版免登录，适合内网日常开发
              </li>
              <li>
                <CheckCircle2 />
                公网版统一认证与设备访问校验
              </li>
              <li>
                <CheckCircle2 />
                前后台共享设计规范，管理操作可追溯
              </li>
            </ul>
            <Button asChild variant="outline">
              <Link to="/docs?page=api">
                了解接入方式 <ArrowRight />
              </Link>
            </Button>
          </div>
          <div className="home-access-visual">
            <div className="home-visual-heading">
              <ShieldCheck />
              <span>清晰的访问边界</span>
            </div>
            <div className="home-access-flow">
              <span>浏览器 / ostool CLI</span>
              <ArrowDown />
              <strong>ostool-server</strong>
              <ArrowDown />
              <span>服务器侧设备与串口</span>
            </div>
            <p>
              {publicEdition
                ? "认证、权限和租约检查由服务端执行。"
                : "在可信局域网中，通过同一套 API 直接使用设备。"}
            </p>
          </div>
        </section>
        <section className="home-closing">
          <span className="section-label">START BUILDING</span>
          <h2>下一次构建，从这里开始。</h2>
          <p>找到设备，接入工作流，把时间留给系统本身。</p>
          <div className="hero-actions">
            <Button asChild>
              <a href={startPath}>
                {publicEdition
                  ? user
                    ? "进入个人中心"
                    : "开始使用"
                  : "打开本地控制台"}
                <ArrowRight />
              </a>
            </Button>
            <Button asChild variant="outline">
              <Link to="/devices">查看设备</Link>
            </Button>
          </div>
        </section>
      </main>
    </PublicLayout>
  );
}
