import { useId, useState } from "react";
import { useResource, useList } from "@/api/events";
import { api } from "@/api/client";
import { PagePanel } from "@/components/page-panel";
import {
  TextField,
  TextareaField,
  CheckField,
  Section,
  Notice,
  DataTable,
  Row,
  Cell,
  useAction,
} from "@/components/forms";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import type { SiteSettings as Settings, SiteView } from "@/types/api";
const groups: Record<
  string,
  { key: keyof Settings; label: string; area?: boolean }[]
> = {
  site: [
    { key: "name", label: "站点名称" },
    { key: "short_name", label: "站点简称" },
    { key: "tagline", label: "品牌标语" },
    { key: "description", label: "站点描述", area: true },
    { key: "logo_url", label: "LOGO 路径" },
    { key: "favicon_url", label: "站点图标路径" },
    { key: "copyright", label: "版权文字" },
    { key: "github_url", label: "代码仓库链接" },
  ],
  contact: [
    { key: "support_email", label: "联系邮箱" },
    { key: "support_phone", label: "联系电话" },
    { key: "support_url", label: "支持链接" },
    { key: "operator", label: "运营主体" },
    { key: "address", label: "联系地址" },
    { key: "icp_number", label: "ICP备案号" },
    { key: "icp_url", label: "ICP备案链接" },
    { key: "police_number", label: "公安备案号" },
    { key: "police_url", label: "公安备案链接" },
  ],
  legal: [
    { key: "terms_title", label: "用户协议标题" },
    { key: "terms_body", label: "用户协议正文", area: true },
    { key: "privacy_title", label: "隐私政策标题" },
    { key: "privacy_body", label: "隐私政策正文", area: true },
  ],
  maintenance: [
    { key: "maintenance_message", label: "维护提示内容", area: true },
  ],
};
export default function SiteSettings({ group }: { group: string }) {
  const source = useResource("site");
  return source ? (
    <Editor group={group} source={source} />
  ) : (
    <PagePanel>
      <Skeleton className="h-40 w-full" />
    </PagePanel>
  );
}
function Editor({ source, group }: { source: SiteView; group: string }) {
  const id = useId(),
    task = useAction(),
    history = useList("legal_history");
  const [draft, setDraft] = useState(source),
    [dirty, setDirty] = useState(false),
    [awaiting, setAwaiting] = useState(0);
  // Adopt new snapshots only when no local edits exist.
  const current = dirty || awaiting > source.revision ? draft : source;
  const changed = (key: keyof Settings, value: string | boolean) => {
    setDraft({ ...current, settings: { ...current.settings, [key]: value } });
    setDirty(true);
  };
  const conflict = dirty && draft.revision < source.revision;
  return (
    <PagePanel
      className={group === "legal" ? "legal-settings" : ""}
      footer={
        <>
          <span className="panel-status">
            配置版本 {current.revision}
            {dirty ? " · 尚未保存" : ""}
          </span>
          <div className="actions">
            <Button
              variant="outline"
              disabled={task.pending}
              onClick={() => {
                setDraft(source);
                setDirty(false);
                setAwaiting(0);
              }}
            >
              取消
            </Button>
            <Button
              type="submit"
              form={id}
              disabled={!dirty || task.pending || conflict}
            >
              {task.pending
                ? "保存中…"
                : group === "legal"
                  ? "发布协议"
                  : "保存配置"}
            </Button>
          </div>
        </>
      }
    >
      {conflict && (
        <Notice>
          配置已在其他窗口更新。请先保留草稿，再取消以载入最新版本。
        </Notice>
      )}
      {task.error && <Notice>{task.error}</Notice>}
      <form
        id={id}
        className="settings-form"
        onSubmit={(event) => {
          event.preventDefault();
          void task.run(async () => {
            await api.updateSite(current.revision, current.settings);
            setDraft({ ...current, revision: current.revision + 1 });
            setAwaiting(current.revision + 1);
            // The event projection, rather than an extra GET, acknowledges the saved revision.
            setDirty(false);
          }, "配置已保存");
        }}
      >
        <Section
          title={
            (
              {
                site: "站点与品牌",
                contact: "联系与运营信息",
                legal: "用户协议与隐私政策",
                maintenance: "维护提示",
              } as Record<string, string>
            )[group]
          }
        >
          {group === "maintenance" && (
            <CheckField
              label="显示维护提示"
              checked={current.settings.maintenance}
              onChange={(value) => changed("maintenance", value)}
            />
          )}
          {groups[group]?.map((field) =>
            field.area ? (
              <TextareaField
                key={field.key}
                label={field.label}
                value={String(current.settings[field.key])}
                onValue={(value) => changed(field.key, value)}
              />
            ) : (
              <TextField
                key={field.key}
                label={field.label}
                value={String(current.settings[field.key])}
                onValue={(value) => changed(field.key, value)}
              />
            ),
          )}
        </Section>
      </form>
      {group === "legal" && (
        <Section title="已发布版本">
          <p>发布会保留旧版本，注册同意记录关联到具体版本。</p>
          <DataTable
            headers={["文档", "版本", "发布时间", "摘要"]}
            empty={!history.length}
          >
            {history.map((version) => (
              <Row key={version.kind + version.version}>
                <Cell>{version.title}</Cell>
                <Cell>v{version.version}</Cell>
                <Cell>{new Date(version.published_at).toLocaleString()}</Cell>
                <Cell>
                  <code>{version.content_hash.slice(0, 16)}</code>
                </Cell>
              </Row>
            ))}
          </DataTable>
        </Section>
      )}
    </PagePanel>
  );
}
