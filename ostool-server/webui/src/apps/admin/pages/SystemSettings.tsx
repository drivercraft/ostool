import { PagePanel } from "@/components/page-panel";
import { useSearchParams } from "react-router-dom";
import { useEdition } from "@/edition";
import { useResource } from "@/api/events";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { Notice, ReadonlyInfo } from "@/components/forms";
import SiteSettings from "./SiteSettings";
import { useList } from "@/api/events";
import { DataTable, Row, Cell } from "@/components/forms";
import { Server, Tftp } from "./Settings";

export default function SystemSettings() {
  const publicEdition = useEdition().edition === "public";
  const [params, setParams] = useSearchParams();
  const source = useResource("deployment");
  const deliveries = useList("mail_deliveries");
  const tab = (publicEdition
    ? [
        "site",
        "contact",
        "legal",
        "maintenance",
        "server",
        "tftp",
        "security",
        "email",
        "storage",
      ]
    : ["site", "contact", "legal", "maintenance", "server", "tftp", "storage"]
  ).includes(params.get("tab") ?? "")
    ? params.get("tab")!
    : "site";
  return (
    <>
      <Tabs
        className="workspace-tabs"
        value={tab}
        onValueChange={(value) => setParams({ tab: value })}
      >
        <TabsList variant="line" className="system-tabs">
          <TabsTrigger value="site">站点信息</TabsTrigger>
          <TabsTrigger value="contact">联系与备案</TabsTrigger>
          <TabsTrigger value="legal">协议与隐私</TabsTrigger>
          <TabsTrigger value="maintenance">维护提示</TabsTrigger>
          {publicEdition && <TabsTrigger value="email">邮件服务</TabsTrigger>}
          <TabsTrigger value="server">网络与上传</TabsTrigger>
          <TabsTrigger value="tftp">TFTP 服务</TabsTrigger>
          {publicEdition && (
            <TabsTrigger value="security">登录与部署</TabsTrigger>
          )}
          <TabsTrigger value="storage">数据存储</TabsTrigger>
        </TabsList>
        {["site", "contact", "legal", "maintenance"].map((group) => (
          <TabsContent
            key={group}
            value={group}
            forceMount
            hidden={tab !== group}
          >
            <SiteSettings group={group} />
          </TabsContent>
        ))}
        <TabsContent value="email">
          <PagePanel
            footer={
              <span className="panel-status">
                最近 100 次邮件投递，不记录正文或验证码
              </span>
            }
          >
            {source?.security && <ReadonlyInfo entries={source.security} />}
            <DataTable
              headers={["用途", "收件邮箱", "状态", "时间"]}
              empty={!deliveries.length}
            >
              {deliveries.map((mail) => (
                <Row key={mail.id}>
                  <Cell>{mail.purpose}</Cell>
                  <Cell>{mail.recipient_email}</Cell>
                  <Cell>{mail.status}</Cell>
                  <Cell>{new Date(mail.created_at).toLocaleString()}</Cell>
                </Row>
              ))}
            </DataTable>
          </PagePanel>
        </TabsContent>
        <TabsContent value="server" forceMount hidden={tab !== "server"}>
          <Server />
        </TabsContent>
        <TabsContent value="tftp" forceMount hidden={tab !== "tftp"}>
          <Tftp />
        </TabsContent>
        <TabsContent value="security">
          <PagePanel
            footer={
              <span className="panel-status">部署参数在服务启动时读取</span>
            }
          >
            {source && (
              <ReadonlyInfo
                entries={{
                  登录验证: source.auth_enabled,
                  允许注册: source.allow_registration,
                  公网地址: source.public_base_url,
                  "Access Token 有效期": `${source.access_token_ttl_secs} 秒`,
                  "PostgreSQL 已连接": source.database_connected,
                  "Redis 已连接": source.redis_connected,
                  ...source.security,
                }}
              />
            )}
            <Notice>
              部署时通过 .env、进程环境变量或 systemd EnvironmentFile
              设置数据库连接、公网地址及认证开关。密码与数据库连接串不会发送到浏览器。
            </Notice>
          </PagePanel>
        </TabsContent>
        <TabsContent value="storage">
          <PagePanel
            footer={
              <span className="panel-status">同时备份数据库与文件目录</span>
            }
          >
            {source && <ReadonlyInfo entries={source.storage} />}
            <Notice>
              活动串口和板卡租约只在当前进程有效，服务重启不会恢复硬件占用。请同时备份
              PostgreSQL、TOML 配置和文件数据目录。
            </Notice>
          </PagePanel>
        </TabsContent>
      </Tabs>
    </>
  );
}
