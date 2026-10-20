import { ListToolbar } from "@/components/list-toolbar";
import { useState } from "react";
import { api } from "@/api/client";
import { useList, useResource } from "@/api/events";
import type { ManagedUser, UserInput } from "@/types/api";
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

export default function Users() {
  const users = useList("users"),
    roles = useList("roles"),
    deployment = useResource("deployment");
  const [search, setSearch] = useState("");
  const [roleFilter, setRoleFilter] = useState("");
  const [statusFilter, setStatusFilter] = useState("");
  const [editor, setEditor] = useState<{
    id?: string;
    draft: UserInput;
  } | null>(null);
  const task = useAction();
  const filtered = users.filter(
    (u) =>
      (!roleFilter || u.role_id === roleFilter) &&
      (!statusFilter || String(u.disabled) === statusFilter) &&
      `${u.email} ${u.display_name ?? ""}`
        .toLowerCase()
        .includes(search.toLowerCase()),
  );
  const open = (user?: ManagedUser) =>
    setEditor({
      id: user?.id,
      draft: {
        email: user?.email ?? "",
        display_name: user?.display_name ?? "",
        role_id: user?.role_id ?? "member",
        disabled: user?.disabled ?? false,
      },
    });
  const patch = (value: Partial<UserInput>) =>
    setEditor((e) => (e ? { ...e, draft: { ...e.draft, ...value } } : e));
  return (
    <>
      <header className="page-header">
        <div>
          <h1>用户列表</h1>
          <p className="heading-description">
            管理用户账号、角色、状态与登录凭据。
          </p>
        </div>
      </header>
      {!deployment?.database_connected && (
        <Notice>用户管理需要连接 PostgreSQL。</Notice>
      )}
      <ListToolbar
        actions={
          <Button
            disabled={!deployment?.database_connected}
            onClick={() => open()}
          >
            创建用户
          </Button>
        }
        count={`${filtered.length} 位用户`}
      >
        <TextField
          label="搜索用户"
          value={search}
          placeholder="邮箱或显示名称"
          onValue={setSearch}
        />
        <SelectField
          label="角色筛选"
          value={roleFilter}
          onValue={setRoleFilter}
          options={[
            { value: "", label: "全部角色" },
            ...roles.map((r) => ({ value: r.id, label: r.name })),
          ]}
        />
        <SelectField
          label="账号状态"
          value={statusFilter}
          onValue={setStatusFilter}
          options={[
            { value: "", label: "全部状态" },
            { value: "false", label: "正常" },
            { value: "true", label: "已禁用" },
          ]}
        />
      </ListToolbar>
      <DataTable
        headers={["用户", "角色", "状态", "创建时间", "操作"]}
        empty={!filtered.length}
      >
        {filtered.map((user) => (
          <Row key={user.id}>
            <Cell>
              <strong>{user.display_name || "未设置名称"}</strong>
              <small>{user.email}</small>
            </Cell>
            <Cell>
              {roles.find((r) => r.id === user.role_id)?.name ?? user.role_id}
            </Cell>
            <Cell>
              <span
                className={`user-status ${user.disabled ? "disabled" : "enabled"}`}
              >
                {user.disabled ? "已禁用" : "正常"}
              </span>
            </Cell>
            <Cell>{new Date(user.created_at).toLocaleString()}</Cell>
            <Cell>
              <div className="actions">
                <Button variant="outline" size="sm" onClick={() => open(user)}>
                  编辑
                </Button>
                <ConfirmAction
                  label="删除"
                  description={`永久删除 ${user.email} 及其登录令牌？`}
                  action={() => api.deleteUser(user.id)}
                />
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
          <DialogTitle>{editor?.id ? "编辑用户" : "创建用户"}</DialogTitle>
          <DialogDescription>
            修改角色、密码或状态会撤销该用户已有的登录令牌。不能禁用、删除自己或移除自己的管理权限。
          </DialogDescription>
          {editor && (
            <form
              onSubmit={(e) => {
                e.preventDefault();
                void task
                  .run(
                    () =>
                      editor.id
                        ? api.updateUser(editor.id, editor.draft)
                        : api.createUser(editor.draft),
                    "用户已保存",
                  )
                  .then((ok) => {
                    if (ok) setEditor(null);
                  });
              }}
              className="management-form"
            >
              <TextField
                label="邮箱"
                type="email"
                required
                value={editor.draft.email}
                onValue={(email) => patch({ email })}
              />
              <TextField
                label="显示名称"
                maxLength={120}
                value={editor.draft.display_name ?? ""}
                onValue={(display_name) => patch({ display_name })}
              />
              <SelectField
                label="角色"
                value={editor.draft.role_id}
                onValue={(role_id) => patch({ role_id })}
                options={roles.map((r) => ({ value: r.id, label: r.name }))}
              />
              <TextField
                label={editor.id ? "新密码（留空保留）" : "初始密码"}
                type="password"
                required={!editor.id}
                minLength={12}
                maxLength={1024}
                autoComplete="new-password"
                value={editor.draft.password ?? ""}
                onValue={(password) =>
                  patch({ password: password || undefined })
                }
              />
              <CheckField
                label="禁用账号"
                checked={editor.draft.disabled}
                onChange={(disabled) => patch({ disabled })}
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
                <Button type="submit" disabled={task.pending}>
                  {task.pending ? "保存中…" : "保存用户"}
                </Button>
              </DialogFooter>
            </form>
          )}
        </DialogContent>
      </Dialog>
    </>
  );
}
