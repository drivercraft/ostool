import { useEffect, useId, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { Plus, Trash2, FileText } from "lucide-react";
import { api } from "@/api/client";
import { admin, useList, useResource, useErrors } from "@/api/events";
import type { DeviceDocument, DeviceDocumentView } from "@/types/api";
import { ListToolbar } from "@/components/list-toolbar";
import { PagePanel } from "@/components/page-panel";
import { Button } from "@/components/ui/button";
import {
  TextField,
  TextareaField,
  Section,
  Notice,
  DataTable,
  Row,
  Cell,
  useAction,
} from "@/components/forms";
import { Skeleton } from "@/components/ui/skeleton";

export default function DeviceDocuments() {
  const { boardId } = useParams();
  const boards = useList("boards");
  const summaries = useResource("device_documents") ?? {};
  const selected = useResource("device_document");
  const error = useErrors().device_document;
  const [search, setSearch] = useState("");
  useEffect(() => {
    admin.setDeviceDocument(boardId ?? null);
    return () => admin.setDeviceDocument(null);
  }, [boardId]);
  if (boardId) {
    if (error) return <Notice>{error}</Notice>;
    if (!selected || selected.id !== boardId)
      return <Skeleton className="h-64 w-full" />;
    if (!selected.document)
      return (
        <Notice>
          开发板不存在或已删除。<Link to="/device-documents">返回设备文档</Link>
        </Notice>
      );
    return <DocumentEditor key={boardId} source={selected} />;
  }
  const filtered = boards.filter((board) =>
    `${board.id} ${board.board_type}`
      .toLowerCase()
      .includes(search.toLowerCase()),
  );
  return (
    <PagePanel
      table
      className="device-documents-panel"
      toolbar={
        <ListToolbar count={`共 ${filtered.length} 台设备`}>
          <TextField label="搜索设备文档" value={search} onValue={setSearch} />
        </ListToolbar>
      }
      footer={<span>共 {filtered.length} 台开发板</span>}
    >
      <DataTable
        headers={["开发板", "公开标题", "文档状态", "操作"]}
        empty={!filtered.length}
      >
        {filtered.map((board) => (
          <Row key={board.id}>
            <Cell>
              <strong>{board.id}</strong>
              <small>{board.board_type}</small>
            </Cell>
            <Cell>{summaries[board.id]?.title || "尚未填写"}</Cell>
            <Cell>{summaries[board.id] ? "已发布" : "待编写"}</Cell>
            <Cell>
              <Button asChild permission="devices.manage" variant="outline">
                <Link to={`/device-documents/${encodeURIComponent(board.id)}`}>
                  <FileText />
                  编辑文档
                </Link>
              </Button>
            </Cell>
          </Row>
        ))}
      </DataTable>
    </PagePanel>
  );
}
function DocumentEditor({ source }: { source: DeviceDocumentView }) {
  const [draft, setDraft] = useState<DeviceDocument>(() => source.document!);
  const [revision, setRevision] = useState(source.revision);
  const formId = useId();
  const task = useAction();
  const conflict = source.revision > revision;
  const set = <K extends keyof DeviceDocument>(
    key: K,
    value: DeviceDocument[K],
  ) => setDraft((old) => ({ ...old, [key]: value }));
  return (
    <PagePanel
      className="device-document-editor"
      footer={
        <>
          <span className="panel-status">
            {conflict ? "已有更新，草稿已保留" : "保存后发布到设备详情页"}
          </span>
          <div className="panel-footer-actions">
            <Button asChild variant="outline">
              <a
                href={`/devices/${encodeURIComponent(source.id)}`}
                target="_blank"
                rel="noreferrer"
              >
                查看详情页
              </a>
            </Button>
            <Button asChild variant="outline">
              <Link to="/device-documents">返回文档列表</Link>
            </Button>
            <Button
              form={formId}
              type="submit"
              permission="devices.manage"
              disabled={task.pending || conflict}
            >
              保存并发布
            </Button>
          </div>
        </>
      }
    >
      {conflict && (
        <Notice>
          文档已被其他管理员更新，当前输入已保留。
          <Button
            variant="outline"
            onClick={() => {
              setDraft(source.document!);
              setRevision(source.revision);
            }}
          >
            载入最新文档
          </Button>
        </Notice>
      )}
      {task.error && <Notice>{task.error}</Notice>}
      <form
        className="device-document-form"
        id={formId}
        onSubmit={(event) => {
          event.preventDefault();
          void task.run(async () => {
            const saved = await api.saveDeviceDocument(
              source.id,
              revision,
              draft,
            );
            setRevision(saved.revision);
          }, "设备文档已发布");
        }}
      >
        <Section
          title="公开介绍"
          hint={`${source.id} · 以下内容将公开显示在设备详情页。`}
        >
          <TextField
            label="公开标题"
            value={draft.title}
            onValue={(v) => set("title", v)}
            maxLength={160}
          />
          <TextareaField
            label="设备简介"
            className="device-document-summary"
            value={draft.summary}
            onValue={(v) => set("summary", v)}
            maxLength={600}
          />
          <TextareaField
            label="详细介绍"
            className="device-document-description"
            value={draft.description}
            onValue={(v) => set("description", v)}
            maxLength={16000}
          />
          <TextField
            label="设备图片地址"
            value={draft.image_url}
            onValue={(v) => set("image_url", v)}
            hint="站内图片路径或 HTTPS 图片地址；不填时展示开发板示意图。"
            maxLength={2048}
          />
        </Section>
        <Section title="设备亮点" hint="用简短的文字说明设备特点，最多 8 项。">
          {draft.highlights.map((value, index) => (
            <div className="document-edit-row" key={index}>
              <TextField
                label={`亮点 ${index + 1}`}
                value={value}
                maxLength={240}
                onValue={(v) =>
                  set(
                    "highlights",
                    draft.highlights.map((old, i) => (i === index ? v : old)),
                  )
                }
              />
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={`删除亮点 ${index + 1}`}
                onClick={() =>
                  set(
                    "highlights",
                    draft.highlights.filter((_, i) => i !== index),
                  )
                }
              >
                <Trash2 />
              </Button>
            </div>
          ))}
          <Button
            type="button"
            variant="outline"
            disabled={draft.highlights.length >= 8}
            onClick={() => set("highlights", [...draft.highlights, ""])}
          >
            <Plus />
            添加亮点
          </Button>
        </Section>
        <Section
          title="公开参数"
          hint="填写已确认的硬件规格，参数名称和值成对展示。"
        >
          {draft.specifications.map((item, index) => (
            <div className="document-edit-row" key={index}>
              <TextField
                label={`参数名称 ${index + 1}`}
                value={item.name}
                maxLength={80}
                onValue={(v) =>
                  set(
                    "specifications",
                    draft.specifications.map((old, i) =>
                      i === index ? { ...old, name: v } : old,
                    ),
                  )
                }
              />
              <TextField
                label={`参数值 ${index + 1}`}
                value={item.value}
                maxLength={400}
                onValue={(v) =>
                  set(
                    "specifications",
                    draft.specifications.map((old, i) =>
                      i === index ? { ...old, value: v } : old,
                    ),
                  )
                }
              />
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={`删除参数 ${index + 1}`}
                onClick={() =>
                  set(
                    "specifications",
                    draft.specifications.filter((_, i) => i !== index),
                  )
                }
              >
                <Trash2 />
              </Button>
            </div>
          ))}
          <Button
            type="button"
            variant="outline"
            disabled={draft.specifications.length >= 32}
            onClick={() =>
              set("specifications", [
                ...draft.specifications,
                { name: "", value: "" },
              ])
            }
          >
            <Plus />
            添加参数
          </Button>
        </Section>
      </form>
    </PagePanel>
  );
}
