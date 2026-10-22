import { PagePanel } from "@/components/page-panel";
import {
  useLocation,
  useNavigationType,
  useSearchParams,
} from "react-router-dom";
import { useLayoutEffect, useState } from "react";
import { useEdition } from "@/edition";
import { useResource } from "@/api/events";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { Notice, ReadonlyInfo } from "@/components/forms";
import SiteSettings from "./SiteSettings";
import { Server, Tftp, Accounts } from "./Settings";

export default function SystemSettings() {
  const publicEdition = useEdition().edition === "public";
  const [params, setParams] = useSearchParams();
  const source = useResource("deployment");
  const routeTab = (publicEdition
    ? [
        "site",
        "contact",
        "terms",
        "privacy",
        "maintenance",
        "server",
        "tftp",
        "security",
        "email",
        "storage",
      ]
    : [
        "site",
        "contact",
        "terms",
        "privacy",
        "maintenance",
        "server",
        "tftp",
        "storage",
      ]
  ).includes(params.get("tab") ?? "")
    ? params.get("tab")!
    : "site";
  const location = useLocation();
  const navigationType = useNavigationType();
  const [tab, setTab] = useState(routeTab);
  useLayoutEffect(() => {
    // 标签点击立即更新界面；路由过渡不能将刚选择的标签切回旧值。
    // 浏览器前后导航和外部链接仍按地址恢复标签。
    if (navigationType === "POP" || location.state?.settingsTab !== routeTab) {
      setTab(routeTab);
    }
  }, [location.key, location.state, navigationType, routeTab]);
  return (
    <>
      <Tabs
        className="workspace-tabs system-settings"
        activationMode="manual"
        value={tab}
        onValueChange={(value) => {
          setTab(value);
          setParams({ tab: value }, { state: { settingsTab: value } });
        }}
      >
        <TabsList className="system-tabs" aria-label="系统设置">
          <TabsTrigger value="site">站点信息</TabsTrigger>
          <TabsTrigger value="contact">联系与备案</TabsTrigger>
          <TabsTrigger value="terms">用户协议</TabsTrigger>
          <TabsTrigger value="privacy">隐私政策</TabsTrigger>
          <TabsTrigger value="maintenance">维护提示</TabsTrigger>
          {publicEdition && <TabsTrigger value="email">邮件服务</TabsTrigger>}
          <TabsTrigger value="server">网络与上传</TabsTrigger>
          <TabsTrigger value="tftp">TFTP 服务</TabsTrigger>
          {publicEdition && (
            <TabsTrigger value="security">登录与部署</TabsTrigger>
          )}
          <TabsTrigger value="storage">数据存储</TabsTrigger>
        </TabsList>
        {["site", "contact", "terms", "privacy", "maintenance"].map((group) => (
          <TabsContent
            key={group}
            value={group}
            forceMount
            hidden={tab !== group}
          >
            <SiteSettings group={group} />
          </TabsContent>
        ))}
        {publicEdition && (
          <TabsContent value="email" forceMount hidden={tab !== "email"}>
            <Accounts group="email" />
          </TabsContent>
        )}
        <TabsContent value="server" forceMount hidden={tab !== "server"}>
          <Server />
        </TabsContent>
        <TabsContent value="tftp" forceMount hidden={tab !== "tftp"}>
          <Tftp />
        </TabsContent>
        {publicEdition && (
          <TabsContent value="security" forceMount hidden={tab !== "security"}>
            <Accounts group="security" />
          </TabsContent>
        )}
        <TabsContent value="storage">
          <PagePanel
            footer={
              <span className="panel-status">同时备份数据库与文件目录</span>
            }
          >
            {source && (
              <section className="page-section">
                <h2>数据存储配置</h2>
                <ReadonlyInfo entries={source.storage} />
              </section>
            )}
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
