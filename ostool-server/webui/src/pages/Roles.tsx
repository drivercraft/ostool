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
    users = useList("users"),
    deployment = useResource("deployment");
  const [editor, setEditor] = useState<{
    id?: string;
    builtin: boolean;
    draft: RoleInput;
  } | null>(null);
  const task = useAction();
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
  return (
    <>
      <header className="page-header">
        <div>
          <h1>用户角色</h1>
          <p className="heading-description">
            角色可授予普通板卡访问或完整后台管理权限。内置角色权限固定。
          </p>
        </div>
        <Button
          disabled={!deployment?.database_connected}
          onClick={() => open()}
        >
          创建角色
        </Button>
      </header>
      {!deployment?.database_connected && (
        <Notice>角色管理需要连接 PostgreSQL。</Notice>
      )}
      <DataTable
        headers={["角色", "权限", "用户数", "说明", "操作"]}
        empty={!roles.length}
      >
        {roles.map((role) => (
          <Row key={role.id}>
            <Cell>
              <strong>{role.name}</strong>
              <small>{role.builtin ? "内置角色" : "自定义角色"}</small>
            </Cell>
            <Cell>
              {role.administrator ? "后台全部管理权限" : "板卡会话访问"}
            </Cell>
            <Cell>{users.filter((u) => u.role_id === role.id).length}</Cell>
            <Cell>{role.description || "—"}</Cell>
            <Cell>
              <div className="actions">
                <Button variant="outline" size="sm" onClick={() => open(role)}>
                  编辑
                </Button>
                {!role.builtin && (
                  <ConfirmAction
                    label="删除"
                    disabled={users.some((u) => u.role_id === role.id)}
                    description={`删除角色 ${role.name}？`}
                    action={() => api.deleteRole(role.id)}
                  />
                )}
              </div>
            </Cell>
          </Row>
        ))}
      </DataTable>
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
                  editor.builtin || users.some((u) => u.role_id === editor.id)
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
