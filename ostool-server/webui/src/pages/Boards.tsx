import { useState } from "react";
import { Link } from "react-router-dom";
import { PlusIcon } from "lucide-react";
import { api } from "@/api/client";
import { useList, useResource } from "@/api/events";
import { Button } from "@/components/ui/button";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { Badge } from "@/components/ui/badge";
import { FieldGroup } from "@/components/ui/field";
import {
  DataTable,
  Row,
  Cell,
  ConfirmAction,
  SelectField,
  TextField,
  useAction,
  Notice,
} from "@/components/forms";
export const leaseLabels: Record<string, string> = {
  idle: "空闲",
  using: "使用中",
  releasing: "释放中",
  error: "释放失败",
};
export default function Boards() {
  const boards = useList("boards"),
    loaders = useList("loaders"),
    runtimes = useResource("runtimes") ?? {},
    virtual = useResource("virtual");
  const [search, setSearch] = useState("");
  const [type, setType] = useState(""),
    [tag, setTag] = useState(""),
    [status, setStatus] = useState("");
  const [virtualMac, setVirtualMac] = useState("");
  const task = useAction();
  const quarantined = useList("quarantined_boards");
  const filtered = boards.filter(
    (b) =>
      (!search ||
        `${b.id} ${b.board_type} ${b.network_identity?.mac_address ?? ""}`
          .toLowerCase()
          .includes(search.toLowerCase())) &&
      (!type || b.board_type === type) &&
      (!tag || b.tags.some((t) => t.includes(tag))) &&
      (!status ||
        (b.disabled ? "disabled" : (runtimes[b.id]?.lease_state ?? "idle")) ===
          status),
  );
  return (
    <>
      <header className="page-header">
        <div>
          <h1>开发板</h1>
          <p className="heading-description">
            共 {boards.length} 台 ·{" "}
            {
              boards.filter(
                (b) =>
                  !b.disabled &&
                  (runtimes[b.id]?.lease_state ?? "idle") === "idle",
              ).length
            }{" "}
            台可用
          </p>
        </div>
        <Button asChild>
          <Link to="/boards/new">
            <PlusIcon data-icon="inline-start" />
            新建开发板
          </Link>
        </Button>
      </header>
      {quarantined.length > 0 && (
        <Notice>
          <details>
            <summary>
              {quarantined.length} 份不兼容配置已备份隔离，其余开发板可正常使用
            </summary>
            {quarantined.map((q) => (
              <div key={q.backup_path} className="quarantine-entry">
                <strong>{q.original_path.split("/").pop()}</strong>
                <p>{q.reason}</p>
                <code>{q.backup_path}</code>
              </div>
            ))}
            <p>修正备份文件后放回板卡配置目录，再重启服务加载。</p>
          </details>
        </Notice>
      )}
      <Tabs defaultValue="boards">
        <TabsList variant="line" className="inventory-tabs">
          <TabsTrigger value="boards">
            已配置 <span>{boards.length}</span>
          </TabsTrigger>
          <TabsTrigger value="discovery">
            发现设备{" "}
            <span>
              {loaders.filter((d) => !d.bound_board_id).length} 未绑定
            </span>
          </TabsTrigger>
          {virtual?.enabled && (
            <TabsTrigger value="virtual">
              虚拟设备 <span>{virtual.devices.length}</span>
            </TabsTrigger>
          )}
        </TabsList>
        <TabsContent value="boards">
          <FieldGroup className="filter-bar">
            <TextField
              label="搜索开发板"
              value={search}
              onValue={setSearch}
              placeholder="搜索名称、板型或 MAC"
            />
            <SelectField
              label="板型"
              value={type}
              onValue={setType}
              options={[
                { value: "", label: "全部板型" },
                ...Array.from(new Set(boards.map((b) => b.board_type))).map(
                  (value) => ({ value, label: value }),
                ),
              ]}
            />
            <TextField
              label="标签"
              value={tag}
              onValue={setTag}
              placeholder="筛选标签"
            />
            <SelectField
              label="状态"
              value={status}
              onValue={setStatus}
              options={[
                { value: "", label: "全部状态" },
                ...Object.entries({ ...leaseLabels, disabled: "已禁用" }).map(
                  ([value, label]) => ({ value, label }),
                ),
              ]}
            />
            {(search || type || tag || status) && (
              <Button
                variant="ghost"
                type="button"
                onClick={() => {
                  setSearch("");
                  setType("");
                  setTag("");
                  setStatus("");
                }}
              >
                清除筛选
              </Button>
            )}
            <span className="filter-count">{filtered.length} 台</span>
          </FieldGroup>
          <DataTable
            headers={["开发板", "状态", "连接", "启动方式", "操作"]}
            empty={!filtered.length}
          >
            {filtered.map((b) => (
              <Row key={b.id}>
                <Cell>
                  <Link
                    className="board-name"
                    to={`/boards/${encodeURIComponent(b.id)}`}
                  >
                    {b.id}
                  </Link>
                  <small>
                    {b.board_type}
                    {b.tags.length ? ` · ${b.tags.join(" · ")}` : ""}
                  </small>
                </Cell>
                <Cell>
                  <Badge
                    variant={
                      b.disabled
                        ? "secondary"
                        : runtimes[b.id]?.lease_state === "error"
                          ? "destructive"
                          : runtimes[b.id]?.lease_state === "using"
                            ? "info"
                            : runtimes[b.id]?.lease_state === "releasing"
                              ? "warning"
                              : "success"
                    }
                  >
                    {b.disabled
                      ? "已禁用"
                      : leaseLabels[runtimes[b.id]?.lease_state ?? "idle"]}
                  </Badge>
                  {runtimes[b.id]?.last_release_error && (
                    <small className="text-destructive">
                      {runtimes[b.id].last_release_error}
                    </small>
                  )}
                </Cell>
                <Cell className="mono">
                  {b.boot.kind === "httpboot"
                    ? "自动绑定"
                    : (b.serial?.resolved_device_path ??
                      b.serial?.key.value ??
                      "无串口")}
                  <small>
                    {b.network_identity?.mac_address ?? "未绑定 MAC"}
                  </small>
                </Cell>
                <Cell>
                  {b.boot.kind === "uboot"
                    ? "U-Boot"
                    : b.boot.kind === "pxe"
                      ? "PXE"
                      : "HTTP Boot"}
                </Cell>
                <Cell>
                  <div className="actions">
                    <Button variant="outline" size="sm" asChild>
                      <Link to={`/boards/${encodeURIComponent(b.id)}`}>
                        编辑
                      </Link>
                    </Button>
                    <ConfirmAction
                      label="删除"
                      triggerVariant="ghost"
                      description={`删除开发板 ${b.id} 的配置？`}
                      action={() => api.deleteBoard(b.id)}
                      disabled={
                        !!runtimes[b.id] &&
                        runtimes[b.id].lease_state !== "idle"
                      }
                    />
                  </div>
                </Cell>
              </Row>
            ))}
          </DataTable>
        </TabsContent>
        <TabsContent value="discovery">
          <section className="page-section inventory-panel">
            <h2>已发现设备</h2>
            <p className="hint">
              设备通过网络上报身份。选择设备创建配置，或在新建页手动输入 MAC。
            </p>
            <DataTable
              headers={["MAC / IP", "硬件", "架构 / Loader", "状态", "操作"]}
              empty={!loaders.length}
            >
              {loaders.map((d) => (
                <Row key={d.mac_address}>
                  <Cell className="mono">
                    {d.mac_address}
                    <small>{d.ip_address}</small>
                  </Cell>
                  <Cell>
                    {[d.hardware.manufacturer, d.hardware.product]
                      .filter(Boolean)
                      .join(" / ") || "—"}
                    <small>
                      {[d.hardware.version, d.hardware.serial]
                        .filter(Boolean)
                        .join(" · ")}
                    </small>
                  </Cell>
                  <Cell>
                    {d.arch}
                    <small>{d.loader_version}</small>
                  </Cell>
                  <Cell>
                    <Badge
                      variant={
                        d.conflict
                          ? "destructive"
                          : d.online
                            ? "success"
                            : "secondary"
                      }
                    >
                      {d.conflict ? "MAC 冲突" : d.online ? "在线" : "离线"}
                    </Badge>
                    <small>
                      {d.bound_board_id
                        ? `已绑定 ${d.bound_board_id}`
                        : "未绑定"}
                    </small>
                    <small>
                      最后上报 {new Date(d.last_seen_at).toLocaleString()}
                    </small>
                  </Cell>
                  <Cell>
                    {d.bound_board_id ? (
                      <Link
                        className="text-link"
                        to={`/boards/${encodeURIComponent(d.bound_board_id)}`}
                      >
                        查看配置
                      </Link>
                    ) : (
                      <Button
                        variant="outline"
                        size="sm"
                        asChild
                        disabled={d.conflict}
                      >
                        <Link
                          aria-disabled={d.conflict}
                          to={`/boards/new?mac=${encodeURIComponent(d.mac_address)}`}
                        >
                          创建配置
                        </Link>
                      </Button>
                    )}
                  </Cell>
                </Row>
              ))}
            </DataTable>
          </section>
        </TabsContent>
        {virtual?.enabled && (
          <TabsContent value="virtual">
            <section className="page-section">
              <h2>虚拟设备</h2>
              <FieldGroup className="filters">
                <TextField
                  label="MAC 地址（可选）"
                  value={virtualMac}
                  onValue={setVirtualMac}
                  placeholder="留空自动生成"
                />
                <Button
                  disabled={task.pending}
                  onClick={() =>
                    void task.run(
                      () => api.createVirtualDevice(virtualMac),
                      "已启动虚拟设备，等待真实网络发现",
                    )
                  }
                >
                  启动虚拟设备
                </Button>
              </FieldGroup>
              {task.error && <Notice>{task.error}</Notice>}
              <DataTable
                headers={["设备", "MAC / TAP", "电源 / 串口", "操作"]}
                empty={!virtual.devices.length}
              >
                {virtual.devices.map((d) => (
                  <Row key={d.id}>
                    <Cell className="mono">
                      {d.id}
                      <small>代次 {d.generation}</small>
                    </Cell>
                    <Cell>
                      {d.mac_address}
                      <small>{d.tap}</small>
                    </Cell>
                    <Cell>
                      {d.powered ? "运行中" : "已停止"}
                      <small>
                        {d.serial_connected ? "串口已连接" : "串口未连接"}
                      </small>
                    </Cell>
                    <Cell>
                      <ConfirmAction
                        label="清理"
                        description="停止并清理这个未绑定的虚拟设备？"
                        action={() => api.deleteVirtualDevice(d.id)}
                        disabled={boards.some(
                          (b) =>
                            b.power_management.kind === "qemu" &&
                            b.power_management.virtual_device_id === d.id,
                        )}
                      />
                    </Cell>
                  </Row>
                ))}
              </DataTable>
            </section>
          </TabsContent>
        )}
      </Tabs>
    </>
  );
}
