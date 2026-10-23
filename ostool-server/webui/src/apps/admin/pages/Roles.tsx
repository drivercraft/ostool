import { Pencil as ActionEdit, Trash2 as ActionDelete } from "lucide-react";
import { RowAction } from "@/components/row-action";
import { PagePanel } from "@/components/page-panel";
import { useListPage } from "@/components/list-pagination";
import { ListToolbar } from "@/components/list-toolbar";
import { useState } from "react";
import { api } from "@/api/client";
import { useList, useResource } from "@/api/events";
import type { RoleInput, UserRole } from "@/types/api";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import {
  TextField,
  SelectField,
  CheckField,
  DataTable,
  Row,
  Cell,
  Notice,
  ConfirmAction,
  useAction,
} from "@/components/forms";

export default function Roles() {
  const roles = useList("roles"),
    deployment = useResource("deployment");
  const [editor, setEditor] = useState<{
    id?: string;
    builtin: boolean;
    draft: RoleInput;
  } | null>(null);
  const task = useAction();
  const [search, setSearch] = useState("");
  const [kind, setKind] = useState("");
  const filtered = roles.filter(
    (r) =>
      r.name.toLowerCase().includes(search.toLowerCase()) &&
      (!kind || String(r.administrator) === kind),
  );
  const open = (role?: UserRole) =>
    setEditor({
      id: role?.id,
      builtin: role?.builtin ?? false,
      draft: {
        name: role?.name ?? "",
        description: role?.description ?? "",
        administrator: role?.administrator ?? false,
      },
    });
  const patch = (value: Partial<RoleInput>) =>
    setEditor((e) => (e ? { ...e, draft: { ...e.draft, ...value } } : e));
  const pagination = useListPage(filtered, JSON.stringify([search, kind]));
  return (
    <>
      {deployment && !deployment.database_connected && (
        <Notice>角色管理需要连接 PostgreSQL。</Notice>
      )}
      <PagePanel
        table
        toolbar={
          <ListToolbar
            actions={
              <Button
                disabled={!deployment?.database_connected}
                onClick={() => open()}
              >
                创建角色
              </Button>
            }
            count={`${filtered.length} 个角色`}
          >
            <TextField
              label="搜索角色"
              value={search}
              onValue={setSearch}
              placeholder="角色名称"
            />
            <SelectField
              label="角色权限"
              value={kind}
              onValue={setKind}
              options={[
                { value: "", label: "全部权限" },
                { value: "true", label: "后台管理" },
                { value: "false", label: "板卡访问" },
              ]}
            />
          </ListToolbar>
        }
        footer={pagination.footer}
        scrollKey={pagination.viewKey}
      >
        <DataTable
          headers={["角色", "权限", "用户数", "说明", "更新时间", "操作"]}
          empty={!filtered.length}
        >
          {pagination.rows.map((role) => (
            <Row key={role.id}>
              <Cell>
                <strong>{role.name}</strong>
                <small>{role.builtin ? "内置角色" : "自定义角色"}</small>
              </Cell>
              <Cell>
                {role.administrator ? "后台全部管理权限" : "板卡会话访问"}
              </Cell>
              <Cell>{role.user_count}</Cell>
              <Cell>{role.description || "—"}</Cell>
              <Cell>
                {role.updated_at
                  ? new Date(role.updated_at).toLocaleString()
                  : "—"}
              </Cell>
              <Cell>
                <div className="actions">
                  <RowAction
                    label="编辑"
                    icon={ActionEdit}
                    onClick={() => open(role)}
                  />
                  {!role.builtin && (
                    <ConfirmAction
                      icon={ActionDelete}
                      label="删除"
                      disabled={role.user_count > 0}
                      description={`删除角色 ${role.name}？`}
                      action={() => api.deleteRole(role.id)}
                    />
                  )}
                </div>
              </Cell>
            </Row>
          ))}
        </DataTable>
      </PagePanel>
      <Dialog
        open={!!editor}
        onOpenChange={(v) => {
          if (!v && !task.pending) setEditor(null);
        }}
      >
        <DialogContent>
          <DialogTitle>{editor?.id ? "编辑角色" : "创建角色"}</DialogTitle>
          <DialogDescription>
            管理员可管理全部用户、板卡与设置。已分配用户的角色需先转移用户再修改权限。
          </DialogDescription>
          {editor && (
            <form
              className="management-form"
              onSubmit={(e) => {
                e.preventDefault();
                void task
                  .run(
                    () =>
                      editor.id
                        ? api.updateRole(editor.id, editor.draft)
                        : api.createRole(editor.draft),
                    "角色已保存",
                  )
                  .then((ok) => {
                    if (ok) setEditor(null);
                  });
              }}
            >
              <TextField
                label="角色名称"
                required
                maxLength={80}
                value={editor.draft.name}
                onValue={(name) => patch({ name })}
              />
              <TextField
                label="说明"
                maxLength={500}
                value={editor.draft.description}
                onValue={(description) => patch({ description })}
              />
              <CheckField
                label="授予全部后台管理权限"
                checked={editor.draft.administrator}
                disabled={
                  editor.builtin ||
                  roles.some((r) => r.id === editor.id && r.user_count > 0)
                }
                onChange={(administrator) => patch({ administrator })}
              />
              {task.error && <Notice>{task.error}</Notice>}
              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  disabled={task.pending}
                  onClick={() => setEditor(null)}
                >
                  取消
                </Button>
                <Button disabled={task.pending} type="submit">
                  {task.pending ? "保存中…" : "保存角色"}
                </Button>
              </DialogFooter>
            </form>
          )}
        </DialogContent>
      </Dialog>
    </>
  );
}
