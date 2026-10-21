import { PagePanel } from "@/components/page-panel";
import { useSearchParams } from "react-router-dom";
import { useEdition } from "@/edition";
import { useResource } from "@/api/events";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { Notice, ReadonlyInfo } from "@/components/forms";
import { Server, Tftp } from "./Settings";

export default function SystemSettings() {
  const publicEdition = useEdition().edition === "public";
  const [params, setParams] = useSearchParams();
  const source = useResource("deployment");
  const tab = (publicEdition
    ? ["server", "tftp", "security", "storage"]
    : ["server", "tftp", "storage"]
  ).includes(params.get("tab") ?? "")
    ? params.get("tab")!
    : "server";
  return (
    <>
      <Tabs
        className="workspace-tabs"
        value={tab}
        onValueChange={(value) => setParams({ tab: value })}
      >
        <TabsList variant="line" className="system-tabs">
          <TabsTrigger value="server">网络与上传</TabsTrigger>
          <TabsTrigger value="tftp">TFTP 服务</TabsTrigger>
          {publicEdition && (
            <TabsTrigger value="security">登录与部署</TabsTrigger>
          )}
          <TabsTrigger value="storage">数据存储</TabsTrigger>
        </TabsList>
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
