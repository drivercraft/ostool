import { Copy, UsersRound } from "lucide-react";
import { ListActions } from "@/components/list-actions";
import { Pencil, Trash2 } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { useState } from "react";
import { api } from "@/api/client";
import { useList } from "@/api/events";
import { useListPage } from "@/components/list-pagination";
import { ListToolbar } from "@/components/list-toolbar";
import { PagePanel } from "@/components/page-panel";
import { RowAction } from "@/components/row-action";
import { EnableAction } from "@/components/enable-action";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  TextField,
  SelectField,
  DataTable,
  Row,
  Cell,
  ConfirmAction,
} from "@/components/forms";
import { usePermissions } from "@/components/permissions";
export default function Roles() {
  const navigate = useNavigate();
  const roles = useList("roles");
  const { can, user } = usePermissions();
  const [search, setSearch] = useState("");
  const [kind, setKind] = useState("");
  const [status, setStatus] = useState("");
  const filtered = roles.filter(
    (r) =>
      (r.code + " " + r.name + " " + r.description)
        .toLowerCase()
        .includes(search.toLowerCase()) &&
      (!kind ||
        (kind === "admin"
          ? r.administrator
          : kind === "instant"
            ? r.permissions.includes("board.instant")
            : !r.builtin)) &&
      (!status || r.enabled === (status === "enabled")),
  );
  const pagination = useListPage(
    filtered,
    JSON.stringify([search, kind, status]),
  );
  return (
    <PagePanel
      table
      toolbar={
        <ListToolbar
          actions={
            can("roles.manage") && (
              <Button onClick={() => navigate("/users/roles/new")}>
                创建角色
              </Button>
            )
          }
          count={`${filtered.length} 个角色`}
        >
          <TextField
            label="搜索角色"
            value={search}
            onValue={setSearch}
            placeholder="角色编码、名称或说明"
          />
          <SelectField
            label="角色权限"
            value={kind}
            onValue={setKind}
            options={[
              { value: "", label: "全部角色" },
              { value: "admin", label: "全部后台管理" },
              { value: "custom", label: "自定义角色" },
              { value: "instant", label: "免审批使用设备" },
            ]}
          />
          <SelectField
            label="角色状态"
            value={status}
            onValue={setStatus}
            options={[
              { value: "", label: "全部状态" },
              { value: "enabled", label: "已启用" },
              { value: "disabled", label: "已停用" },
            ]}
          />
        </ListToolbar>
      }
      footer={pagination.footer}
      scrollKey={pagination.viewKey}
    >
      <DataTable
        rowOffset={pagination.rowOffset}
        headers={[
          "角色",
          "唯一编码",
          "状态",
          "权限",
          "用户数",
          "说明",
          "更新时间",
          "操作",
        ]}
        empty={!filtered.length}
      >
        {pagination.rows.map((role) => (
          <Row key={role.id}>
            <Cell>
              <strong>{role.name}</strong>
              <small>{role.builtin ? "内置角色" : "自定义角色"}</small>
            </Cell>
            <Cell className="mono">{role.code}</Cell>
            <Cell>
              <Badge variant={role.enabled ? "success" : "secondary"}>
                {role.enabled ? "已启用" : "已停用"}
              </Badge>
            </Cell>
            <Cell>
              <Badge variant={role.administrator ? "info" : "secondary"}>
                {role.administrator
                  ? "全部管理权限"
                  : `${role.permissions.length} 项权限`}
              </Badge>
              {role.permissions.includes("board.instant") && (
                <small>
                  免审批使用：{role.allocation_policy.max_duration_minutes} 分钟
                  / 同板型最多 {role.allocation_policy.max_concurrent_sessions}{" "}
                  台
                </small>
              )}
            </Cell>
            <Cell>{role.user_count}</Cell>
            <Cell>{role.description || "—"}</Cell>
            <Cell>
              {role.updated_at
                ? new Date(role.updated_at).toLocaleString()
                : "—"}
            </Cell>
            <Cell>
              <ListActions
                edit={
                  <RowAction
                    label={can("roles.manage") ? "修改" : "查看"}
                    icon={Pencil}
                    to={`/users/roles/${encodeURIComponent(role.id)}`}
                  />
                }
                enable={
                  <EnableAction
                    enabled={role.enabled}
                    name={`角色 ${role.name}`}
                    permission="roles.manage"
                    disabled={role.builtin || user?.role_id === role.id}
                    action={(enabled) =>
                      api.updateRole(role.id, {
                        revision: role.revision,
                        code: role.code,
                        name: role.name,
                        description: role.description,
                        permissions: role.permissions,
                        allocation_policy: role.allocation_policy,
                        enabled,
                      })
                    }
                  />
                }
              >
                <RowAction
                  label="基于此新建"
                  icon={Copy}
                  permission="roles.manage"
                  to={`/users/roles/new?copy=${encodeURIComponent(role.id)}`}
                />
                <RowAction
                  label="转到用户"
                  icon={UsersRound}
                  permission="users.read"
                  to={`/users?role=${encodeURIComponent(role.id)}`}
                />
                {!role.builtin && can("roles.manage") && (
                  <ConfirmAction
                    label="删除"
                    icon={Trash2}
                    disabled={role.user_count > 0}
                    description={`删除角色 ${role.name}？已有用户的角色不能删除。`}
                    action={() => api.deleteRole(role.id)}
                  />
                )}
              </ListActions>
            </Cell>
          </Row>
        ))}
      </DataTable>
    </PagePanel>
  );
}
