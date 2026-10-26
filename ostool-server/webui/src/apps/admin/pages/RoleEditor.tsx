import { useId, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { useList } from "@/api/events";
import { api } from "@/api/client";
import type { RoleInput, UserRole, PermissionCapability } from "@/types/api";
import { PagePanel } from "@/components/page-panel";
import {
  TextField,
  TextareaField,
  Section,
  Notice,
  useAction,
} from "@/components/forms";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Skeleton } from "@/components/ui/skeleton";
import { usePermissions } from "@/components/permissions";
export default function RoleEditor() {
  const { roleId } = useParams();
  const roles = useList("roles"),
    catalog = useList("permissions");
  const role = roles.find((r) => r.id === roleId);
  if (!catalog.length) return <Skeleton className="h-40 w-full" />;
  if (roleId && !role)
    return (
      <Notice>
        角色不存在或已删除。<Link to="/users/roles">返回角色列表</Link>
      </Notice>
    );
  return <Editor key={roleId ?? "new"} role={role} catalog={catalog} />;
}
function Editor({
  role,
  catalog,
}: {
  role?: UserRole;
  catalog: PermissionCapability[];
}) {
  const { can, user } = usePermissions(),
    navigate = useNavigate(),
    id = useId(),
    task = useAction();
  const initial = (): RoleInput => ({
    revision: role?.revision,
    name: role?.name ?? "",
    description: role?.description ?? "",
    permissions:
      role?.permissions ?? ["board.use", "rental.request"].filter(can),
    allocation_policy: role?.allocation_policy ??
      user?.allocation_policy ?? {
        max_duration_minutes: 120,
        max_concurrent_sessions: 1,
        min_available_minutes: 10,
        board_types: [],
      },
  });
  const [draft, setDraft] = useState<RoleInput>(initial),
    [baseline, setBaseline] = useState(() => JSON.stringify(initial())),
    [search, setSearch] = useState("");
  const [types, setTypes] = useState(() =>
    initial().allocation_policy.board_types.join(", "),
  );
  const current = JSON.stringify(initial()),
    conflict = !!role && current !== baseline;
  const readonly = !can("roles.manage"),
    fixed =
      readonly || !!role?.builtin || (!!role && role.id === user?.role_id);
  const selected = new Set(
    draft.permissions.includes("system.admin")
      ? catalog.map((p) => p.code)
      : draft.permissions,
  );
  const patch = (value: Partial<RoleInput>) =>
    setDraft((old) => ({ ...old, ...value }));
  const policy = (key: keyof RoleInput["allocation_policy"], value: unknown) =>
    patch({ allocation_policy: { ...draft.allocation_policy, [key]: value } });
  function choose(codes: string[], checked: boolean) {
    const next = new Set(draft.permissions);
    for (const code of codes.filter(can))
      if (checked) next.add(code);
      else next.delete(code);
    if (checked) {
      let changed = true;
      while (changed) {
        changed = false;
        for (const item of catalog)
          if (next.has(item.code))
            for (const requirement of item.requires)
              if (!next.has(requirement)) {
                next.add(requirement);
                changed = true;
              }
      }
    } else {
      let changed = true;
      while (changed) {
        changed = false;
        for (const item of catalog)
          if (next.has(item.code) && item.requires.some((p) => !next.has(p))) {
            next.delete(item.code);
            changed = true;
          }
      }
    }
    patch({ permissions: [...next].sort() });
  }
  const groups = [...new Set(catalog.map((p) => p.group))];
  const all = catalog.filter((p) => can(p.code)).map((p) => p.code);
  const count = all.filter((p) => selected.has(p)).length;
  return (
    <PagePanel
      toolbar={
        <div className="role-permission-toolbar">
          <span>
            {role ? role.name : "创建角色"} · 已选 {selected.size} 项权限
          </span>
          <TextField
            label="搜索权限"
            value={search}
            onValue={setSearch}
            placeholder="功能、权限名称或说明"
          />
        </div>
      }
      footer={
        <>
          <span className="panel-status">
            {role?.builtin
              ? "内置角色权限只读"
              : conflict
                ? "角色已被其他人更新，草稿已保留"
                : readonly
                  ? "只读权限"
                  : JSON.stringify(draft) !== baseline
                    ? "尚未保存"
                    : "配置已保存"}
          </span>
          <div className="panel-footer-actions">
            <Button variant="outline" asChild>
              <Link to="/users/roles">取消</Link>
            </Button>
            {!readonly && (
              <Button
                form={id}
                type="submit"
                disabled={task.pending || conflict}
              >
                {task.pending ? "保存中…" : "保存角色"}
              </Button>
            )}
          </div>
        </>
      }
    >
      {conflict && (
        <Notice>
          角色配置已更新，草稿已保留。
          <Button
            variant="outline"
            onClick={() => {
              setDraft(initial());
              setTypes(initial().allocation_policy.board_types.join(", "));
              setBaseline(current);
            }}
          >
            载入服务器版本
          </Button>
        </Notice>
      )}
      {task.error && <Notice variant="destructive">{task.error}</Notice>}
      <form
        id={id}
        onSubmit={(event) => {
          event.preventDefault();
          if (!conflict && !readonly)
            void task
              .run(
                () =>
                  role ? api.updateRole(role.id, draft) : api.createRole(draft),
                "角色已保存",
              )
              .then((ok) => {
                if (ok) navigate("/users/roles");
              });
        }}
      >
        <fieldset disabled={readonly || task.pending}>
          <Section title="角色信息">
            <TextField
              label="角色名称"
              maxLength={80}
              required
              value={draft.name}
              onValue={(name) => patch({ name })}
            />
            <TextareaField
              label="说明"
              maxLength={500}
              value={draft.description}
              onValue={(description) => patch({ description })}
            />
          </Section>
        </fieldset>
        <Section title="角色权限">
          <div className="permission-master">
            <span>
              依赖的查看权限会随操作权限一同开启；移除前置权限会一并移除依赖项。
            </span>
            <label>
              <Checkbox
                aria-label="全部权限"
                disabled={fixed}
                checked={
                  count === all.length ? true : count ? "indeterminate" : false
                }
                onCheckedChange={(checked) => choose(all, checked === true)}
              />
              全部开启
            </label>
          </div>
          <fieldset
            disabled={fixed || task.pending}
            className="role-permission-matrix"
          >
            {groups.map((group) => {
              const items = catalog.filter(
                (item) =>
                  item.group === group &&
                  (item.name + item.description + item.code)
                    .toLowerCase()
                    .includes(search.toLowerCase()),
              );
              if (!items.length) return null;
              const active = items.filter((item) =>
                selected.has(item.code),
              ).length;
              return (
                <section className="role-permission-row" key={group}>
                  <div className="role-permission-category">
                    <div>
                      <strong>{group}</strong>
                      <small>
                        {active} / {items.length} 项
                      </small>
                    </div>
                    <Checkbox
                      aria-label={`${group}全部权限`}
                      checked={
                        active === items.length
                          ? true
                          : active
                            ? "indeterminate"
                            : false
                      }
                      onCheckedChange={(checked) =>
                        choose(
                          items.map((p) => p.code),
                          checked === true,
                        )
                      }
                    />
                  </div>
                  <div className="role-permission-options">
                    {items.map((item) => (
                      <label key={item.code} className="role-permission-option">
                        <Checkbox
                          aria-label={item.name}
                          disabled={!can(item.code)}
                          checked={selected.has(item.code)}
                          onCheckedChange={(checked) =>
                            choose([item.code], checked === true)
                          }
                        />
                        <span>
                          <strong>{item.name}</strong>
                          <small>{item.description}</small>
                        </span>
                      </label>
                    ))}
                  </div>
                </section>
              );
            })}
          </fieldset>
        </Section>
        {draft.permissions.includes("board.instant") &&
          !draft.permissions.includes("system.admin") && (
            <fieldset disabled={fixed || task.pending}>
              <Section title="即时使用限制">
                <TextField
                  label="单次最长使用（分钟）"
                  type="number"
                  min={1}
                  max={1440}
                  required
                  value={draft.allocation_policy.max_duration_minutes}
                  onValue={(v) => policy("max_duration_minutes", Number(v))}
                  placeholder="1–1440 分钟"
                />
                <TextField
                  label="每用户并发设备数"
                  type="number"
                  min={1}
                  max={16}
                  required
                  value={draft.allocation_policy.max_concurrent_sessions}
                  onValue={(v) => policy("max_concurrent_sessions", Number(v))}
                  placeholder="1–16 台"
                />
                <TextField
                  label="最短可用时长（分钟）"
                  type="number"
                  min={1}
                  max={draft.allocation_policy.max_duration_minutes}
                  required
                  value={draft.allocation_policy.min_available_minutes}
                  onValue={(v) => policy("min_available_minutes", Number(v))}
                  placeholder="预约临近时，剩余时间不足此值则不分配"
                />
                <TextareaField
                  label="可用板型范围"
                  value={types}
                  onValue={(value) => {
                    setTypes(value);
                    policy(
                      "board_types",
                      value
                        .split(/[,，\n]/)
                        .map((v) => v.trim())
                        .filter(Boolean),
                    );
                  }}
                  placeholder="用逗号或换行分隔；留空允许所有已开放即时使用的设备。"
                />
              </Section>
            </fieldset>
          )}
      </form>
    </PagePanel>
  );
}
