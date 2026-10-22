import { useEffect, useId, useState } from "react";
import { Eye, History, Maximize, Minimize, RotateCcw } from "lucide-react";
import { useResource, useList } from "@/api/events";
import { api } from "@/api/client";
import { PagePanel } from "@/components/page-panel";
import { RowAction } from "@/components/row-action";
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
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import type {
  SiteSettings as Settings,
  SiteView,
  LegalDocument,
} from "@/types/api";

type Field = {
  key: keyof Settings;
  label: string;
  hint: string;
  placeholder?: string;
  area?: boolean;
  type?: "email" | "url";
  maxLength?: number;
};
const groups: Record<string, { title: string; fields: Field[] }[]> = {
  site: [
    {
      title: "站点与品牌",
      fields: [
        {
          key: "name",
          label: "站点名称",
          hint: "用于浏览器标题、页脚和登录页面，最多 80 个字符。",
          placeholder: "例如：实验室开发板平台",
          maxLength: 80,
        },
        {
          key: "short_name",
          label: "站点简称",
          hint: "用于导航栏标志旁的短名称，最多 20 个字符。",
          placeholder: "例如：ostool",
          maxLength: 20,
        },
        {
          key: "tagline",
          label: "品牌标语",
          hint: "显示在站点标志和介绍区域，最多 80 个字符。",
          placeholder: "例如：连接构建、启动与调试",
          maxLength: 80,
        },
        {
          key: "description",
          label: "站点描述",
          hint: "简要介绍平台用途，最多 500 个字符。",
          area: true,
          maxLength: 500,
        },
        {
          key: "logo_url",
          label: "LOGO 路径",
          hint: "同源绝对路径，支持 SVG、PNG；留空使用默认标志。",
          placeholder: "/branding/logo.svg",
          maxLength: 1000,
        },
        {
          key: "favicon_url",
          label: "站点图标路径",
          hint: "浏览器标签页图标的同源路径，支持 ICO、SVG、PNG。",
          placeholder: "/branding/favicon.ico",
          maxLength: 1000,
        },
      ],
    },
    {
      title: "版权与链接",
      fields: [
        {
          key: "copyright",
          label: "版权文字",
          hint: "页脚自动添加当前年份，无需重复填写 © 和年份。",
          placeholder: "例如：某某实验室",
          maxLength: 200,
        },
        {
          key: "github_url",
          label: "代码仓库链接",
          hint: "完整 HTTP(S) 地址；留空隐藏该入口。",
          type: "url",
          placeholder: "https://github.com/drivercraft/ostool",
        },
      ],
    },
  ],
  contact: [
    {
      title: "联系与支持",
      fields: [
        {
          key: "support_email",
          label: "联系邮箱",
          hint: "对外公开的支持邮箱，留空不显示。",
          type: "email",
          placeholder: "support@example.com",
        },
        {
          key: "support_phone",
          label: "联系电话",
          hint: "可包含国家区号、分机，最多 40 个字符。",
          placeholder: "+86 010-12345678",
          maxLength: 40,
        },
        {
          key: "support_work_hours",
          label: "服务时间",
          hint: "对外公开的服务时段，最多 80 个字符。",
          placeholder: "工作日 09:00–18:00",
          maxLength: 80,
        },
        {
          key: "support_url",
          label: "支持链接",
          hint: "完整 HTTP(S) 地址，例如帮助中心或工单入口。",
          type: "url",
          placeholder: "https://example.com/support",
        },
        {
          key: "privacy_contact_email",
          label: "隐私事务邮箱",
          hint: "处理隐私咨询的邮箱；留空时隐私政策页使用联系邮箱。",
          type: "email",
          placeholder: "privacy@example.com",
        },
      ],
    },
    {
      title: "运营与备案",
      fields: [
        {
          key: "operator",
          label: "运营主体",
          hint: "组织或公司名称，用于页脚与运营信息展示。",
          maxLength: 120,
        },
        {
          key: "address",
          label: "联系地址",
          hint: "公开联系地址，最多 300 个字符。",
          maxLength: 300,
        },
        {
          key: "icp_number",
          label: "ICP备案号",
          hint: "按实际备案填写；留空不显示。",
          placeholder: "京ICP备12345678号",
          maxLength: 64,
        },
        {
          key: "icp_url",
          label: "ICP备案链接",
          hint: "备案查询页面的完整 HTTPS 地址。",
          type: "url",
          placeholder: "https://beian.miit.gov.cn/",
        },
        {
          key: "police_number",
          label: "公安备案号",
          hint: "按实际公安联网备案填写；留空不显示。",
          maxLength: 64,
        },
        {
          key: "police_url",
          label: "公安备案链接",
          hint: "该备案的官方查询页面，需为完整 HTTP(S) 地址。",
          type: "url",
          placeholder: "https://www.beian.gov.cn/",
        },
      ],
    },
  ],
  maintenance: [
    {
      title: "维护提示",
      fields: [
        {
          key: "maintenance_message",
          label: "维护提示内容",
          hint: "最多 500 个字符；只展示提示，不中断登录或设备使用。",
          area: true,
          maxLength: 500,
        },
      ],
    },
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
  const legal = group === "terms" || group === "privacy";
  const titleKey = group === "privacy" ? "privacy_title" : "terms_title";
  const bodyKey = group === "privacy" ? "privacy_body" : "terms_body";
  const label = group === "privacy" ? "隐私政策" : "用户协议";
  const [baseline, setBaseline] = useState(source.settings);
  const [draft, setDraft] = useState<Partial<Settings>>({});
  const [awaiting, setAwaiting] = useState(0);
  const [preview, setPreview] = useState(false),
    [fullscreen, setFullscreen] = useState(false),
    [historyOpen, setHistoryOpen] = useState(false);
  const [version, setVersion] = useState<LegalDocument>();
  const [reading, setReading] = useState(false);
  const dirty = Object.keys(draft).length > 0 && !awaiting;
  const current = {
    ...source.settings,
    ...(dirty || awaiting > source.revision ? draft : {}),
  };
  const conflict =
    dirty &&
    (Object.keys(draft) as (keyof Settings)[]).some(
      (key) => source.settings[key] !== baseline[key],
    );
  useEffect(() => {
    if (awaiting && source.revision >= awaiting) {
      setDraft({});
      setAwaiting(0);
    }
  }, [source.revision, awaiting]);
  useEffect(() => {
    if (!fullscreen) return;
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !historyOpen) setFullscreen(false);
    };
    document.addEventListener("keydown", escape);
    return () => document.removeEventListener("keydown", escape);
  }, [fullscreen, historyOpen]);
  function changed(key: keyof Settings, value: string | boolean) {
    if (awaiting) {
      setAwaiting(0);
      setBaseline(current);
    } else if (!dirty) setBaseline(source.settings);
    setDraft((valueDraft) => ({ ...valueDraft, [key]: value }));
  }
  async function readVersion(number: number, restore = false) {
    setReading(true);
    try {
      const document = await api.legalVersion(
        group as "terms" | "privacy",
        number,
      );
      setVersion(document);
      if (restore) {
        setBaseline(source.settings);
        setDraft({ [titleKey]: document.title, [bodyKey]: document.body });
        setHistoryOpen(false);
      }
    } catch (error) {
      void task.run(() => Promise.reject(error));
    } finally {
      setReading(false);
    }
  }
  return (
    <PagePanel
      className={`${legal ? "legal-settings" : ""}${fullscreen ? " legal-fullscreen" : ""}`}
      footer={
        <>
          <span className="panel-status">
            配置版本 {source.revision}
            {dirty ? " · 尚未保存" : ""}
            {legal ? " · 纯文本，最多 100 KB" : ""}
          </span>
          <div className="panel-footer-actions">
            <Button
              variant="outline"
              disabled={task.pending}
              onClick={() => {
                setDraft({});
                setAwaiting(0);
                setBaseline(source.settings);
              }}
            >
              取消
            </Button>
            <Button
              type="submit"
              form={id}
              disabled={!dirty || task.pending || conflict}
            >
              {task.pending ? "保存中…" : legal ? "发布协议" : "保存配置"}
            </Button>
          </div>
        </>
      }
    >
      {conflict && (
        <Notice>
          当前配置已在其他窗口更新，草稿已保留；请先复制草稿，再取消以载入最新版本。
        </Notice>
      )}
      {task.error && <Notice>{task.error}</Notice>}
      <form
        id={id}
        className={legal ? "legal-editor-form" : "settings-form"}
        onSubmit={(event) => {
          event.preventDefault();
          if (conflict) return;
          if (
            legal &&
            new TextEncoder().encode(current[bodyKey]).length > 100000
          ) {
            void task.run(() =>
              Promise.reject(new Error("协议正文不能超过 100 KB")),
            );
            return;
          }
          void task.run(
            async () => {
              await api.updateSite(source.revision, current);
              setAwaiting(source.revision + 1);
            },
            legal ? `${label}已发布` : "配置已保存",
          );
        }}
      >
        {legal ? (
          <>
            <Input
              aria-label={`${label}标题`}
              value={current[titleKey]}
              maxLength={80}
              disabled={task.pending}
              required
              placeholder="文档标题，最多 80 个字符"
              onChange={(event) => changed(titleKey, event.target.value)}
            />
            <div
              className={`legal-editor-workspace${preview ? " has-preview" : ""}`}
            >
              <div className="legal-source">
                <div
                  className="legal-editor-toolbar"
                  role="toolbar"
                  aria-label={`${label}编辑工具栏`}
                >
                  <div className="legal-editor-tools">
                    <span className="panel-status">
                      已发布 v
                      {group === "privacy"
                        ? source.privacy_version
                        : source.terms_version}
                    </span>
                    <RowAction
                      label={preview ? "关闭预览" : "预览"}
                      icon={Eye}
                      aria-pressed={preview}
                      onClick={() => setPreview((value) => !value)}
                    />
                    <RowAction
                      label="发布历史"
                      icon={History}
                      onClick={() => {
                        setVersion(undefined);
                        setHistoryOpen(true);
                      }}
                    />
                    <RowAction
                      label={fullscreen ? "退出全屏" : "全屏"}
                      icon={fullscreen ? Minimize : Maximize}
                      onClick={() => setFullscreen((value) => !value)}
                    />
                  </div>
                </div>
                <textarea
                  aria-label={`${label}正文`}
                  id={`${id}-body`}
                  data-slot="textarea"
                  disabled={task.pending}
                  value={current[bodyKey]}
                  required
                  placeholder="请输入正文；支持换行，前台按纯文本安全展示。发布会生成新的不可变版本。"
                  onChange={(event) => changed(bodyKey, event.target.value)}
                />
              </div>
              {preview && (
                <article className="legal-preview" aria-label={`${label}预览`}>
                  <h1>{current[titleKey]}</h1>
                  <div>{current[bodyKey]}</div>
                </article>
              )}
            </div>
          </>
        ) : (
          groups[group]?.map((section) => (
            <Section key={section.title} title={section.title}>
              {group === "maintenance" && (
                <CheckField
                  label="显示维护提示"
                  checked={current.maintenance}
                  onChange={(value) => changed("maintenance", value)}
                  hint="开启后在前台顶部展示提示，关闭后隐藏。"
                />
              )}
              {section.fields.map((field) =>
                field.area ? (
                  <TextareaField
                    key={field.key}
                    label={field.label}
                    hint={field.hint}
                    placeholder={field.placeholder}
                    maxLength={field.maxLength}
                    value={String(current[field.key] ?? "")}
                    onValue={(value) => changed(field.key, value)}
                  />
                ) : (
                  <TextField
                    key={field.key}
                    label={field.label}
                    hint={field.hint}
                    placeholder={field.placeholder}
                    type={field.type}
                    maxLength={field.maxLength}
                    value={String(current[field.key] ?? "")}
                    onValue={(value) => changed(field.key, value)}
                    required={
                      field.key === "name" || field.key === "short_name"
                    }
                  />
                ),
              )}
            </Section>
          ))
        )}
      </form>
      {legal && (
        <Dialog open={historyOpen} onOpenChange={setHistoryOpen}>
          <DialogContent className="legal-history-dialog">
            <DialogTitle>{label}发布历史</DialogTitle>
            <DialogDescription>
              仅显示当前文档的版本。载入旧版本仅修改草稿，需发布后才生效，原版本和同意记录保持不变。
            </DialogDescription>
            <div className="legal-history-scroll">
              <DataTable
                layout="table"
                headers={["版本", "标题", "发布时间", "操作"]}
                empty={!history.some((item) => item.kind === group)}
              >
                {history
                  .filter((item) => item.kind === group)
                  .map((item) => (
                    <Row key={item.version}>
                      <Cell>v{item.version}</Cell>
                      <Cell>{item.title}</Cell>
                      <Cell>
                        {new Date(item.published_at).toLocaleString()}
                      </Cell>
                      <Cell>
                        <div className="actions">
                          <RowAction
                            label={`查看版本 v${item.version}`}
                            icon={Eye}
                            disabled={reading}
                            onClick={() => void readVersion(item.version)}
                          />
                          <RowAction
                            label={`载入版本 v${item.version}`}
                            icon={RotateCcw}
                            disabled={reading}
                            onClick={() => void readVersion(item.version, true)}
                          />
                        </div>
                      </Cell>
                    </Row>
                  ))}
              </DataTable>
              {version && (
                <article className="legal-preview" aria-label="历史版本正文">
                  <h2>
                    {version.title} · v{version.version}
                  </h2>
                  <div>{version.body}</div>
                </article>
              )}
            </div>
          </DialogContent>
        </Dialog>
      )}
    </PagePanel>
  );
}
