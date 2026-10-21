import {
  Pencil as ActionEdit,
  Trash2 as ActionDelete,
  Eye as ActionView,
  Plus as ActionCreate,
} from "lucide-react";
import { RowAction } from "@/components/row-action";
import { PagePanel } from "@/components/page-panel";
import { useListPage } from "@/components/list-pagination";
import { ListToolbar } from "@/components/list-toolbar";
import { useState } from "react";
import { Link } from "react-router-dom";
import { ArrowLeft, Radar, Monitor, PlusIcon } from "lucide-react";
import { api } from "@/api/client";
import { useList, useResource } from "@/api/events";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent } from "@/components/ui/tabs";
import { Badge } from "@/components/ui/badge";
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
  const [view, setView] = useState("boards");
  const [search, setSearch] = useState("");
  const [type, setType] = useState(""),
    [tag, setTag] = useState(""),
    [status, setStatus] = useState("");
  const [virtualMac, setVirtualMac] = useState("");
  const [discoverySearch, setDiscoverySearch] = useState("");
  const [discoveryStatus, setDiscoveryStatus] = useState("");
  const [virtualSearch, setVirtualSearch] = useState("");
  const filteredLoaders = loaders.filter(
    (d) =>
      `${d.mac_address} ${d.ip_address} ${d.hardware.product ?? ""}`
        .toLowerCase()
        .includes(discoverySearch.toLowerCase()) &&
      (!discoveryStatus ||
        (d.conflict ? "conflict" : d.online ? "online" : "offline") ===
          discoveryStatus),
  );
  const filteredVirtual =
    virtual?.devices.filter((d) =>
      `${d.id} ${d.mac_address} ${d.tap}`
        .toLowerCase()
        .includes(virtualSearch.toLowerCase()),
    ) ?? [];
  const task = useAction();
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
  const boardPage = useListPage(
    filtered,
    JSON.stringify([search, type, tag, status]),
  );
  const discoveryPage = useListPage(
    filteredLoaders,
    JSON.stringify([discoverySearch, discoveryStatus]),
  );
  const virtualPage = useListPage(filteredVirtual, virtualSearch);
  return (
    <>
      <Tabs
        value={view}
        onValueChange={setView}
        className="workspace-tabs board-workspace"
      >
        <TabsContent value="boards" aria-label="开发板列表">
          <PagePanel
            layoutKey="boards"
            table
            toolbar={
              <ListToolbar
                actions={
                  <>
                    <Button asChild>
                      <Link to="/boards/new">
                        <PlusIcon data-icon="inline-start" />
                        新建开发板
                      </Link>
                    </Button>
                    <Button
                      variant="outline"
                      onClick={() => setView("discovery")}
                    >
                      <Radar size={16} />
                      发现设备
                      {loaders.some((d) => !d.bound_board_id) && (
                        <Badge variant="secondary">
                          {loaders.filter((d) => !d.bound_board_id).length}
                        </Badge>
                      )}
                    </Button>
                    {virtual?.enabled && (
                      <Button
                        variant="outline"
                        onClick={() => setView("virtual")}
                      >
                        <Monitor size={16} />
                        虚拟设备
                      </Button>
                    )}
                  </>
                }
                count={`${filtered.length} 台`}
              >
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
                    ...Object.entries({
                      ...leaseLabels,
                      disabled: "已禁用",
                    }).map(([value, label]) => ({ value, label })),
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
              </ListToolbar>
            }
            footer={boardPage.footer}
            scrollKey={boardPage.viewKey}
          >
            <DataTable
              headers={["开发板", "状态", "连接", "启动方式", "操作"]}
              empty={!filtered.length}
            >
              {boardPage.rows.map((b) => (
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
                      <RowAction
                        label="编辑"
                        icon={ActionEdit}
                        to={`/boards/${encodeURIComponent(b.id)}`}
                      />
                      <ConfirmAction
                        icon={ActionDelete}
                        label="删除"
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
          </PagePanel>
        </TabsContent>
        <TabsContent value="discovery" aria-label="发现设备">
          <PagePanel
            layoutKey="discovery"
            table
            toolbar={
              <ListToolbar
                actions={
                  <>
                    <Button variant="outline" onClick={() => setView("boards")}>
                      <ArrowLeft size={16} />
                      开发板列表
                    </Button>
                    <Button asChild variant="outline">
                      <Link to="/boards/new">新建开发板</Link>
                    </Button>
                  </>
                }
                count={`${filteredLoaders.length} 台`}
              >
                <TextField
                  label="搜索发现设备"
                  value={discoverySearch}
                  onValue={setDiscoverySearch}
                  placeholder="MAC、IP 或硬件"
                />
                <SelectField
                  label="发现设备状态"
                  value={discoveryStatus}
                  onValue={setDiscoveryStatus}
                  options={[
                    { value: "", label: "全部状态" },
                    { value: "online", label: "在线" },
                    { value: "offline", label: "离线" },
                    { value: "conflict", label: "MAC 冲突" },
                  ]}
                />
              </ListToolbar>
            }
            footer={discoveryPage.footer}
            scrollKey={discoveryPage.viewKey}
          >
            <DataTable
              headers={["MAC / IP", "硬件", "架构 / Loader", "状态", "操作"]}
              empty={!filteredLoaders.length}
            >
              {discoveryPage.rows.map((d) => (
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
                      <RowAction
                        label="查看配置"
                        icon={ActionView}
                        to={`/boards/${encodeURIComponent(d.bound_board_id)}`}
                      />
                    ) : (
                      <RowAction
                        label="创建配置"
                        icon={ActionCreate}
                        disabled={d.conflict}
                        to={`/boards/new?mac=${encodeURIComponent(d.mac_address)}`}
                      />
                    )}
                  </Cell>
                </Row>
              ))}
            </DataTable>
          </PagePanel>
        </TabsContent>
        {virtual?.enabled && (
          <TabsContent value="virtual">
            <PagePanel
              layoutKey="virtual"
              table
              toolbar={
                <ListToolbar
                  actions={
                    <>
                      <Button
                        variant="outline"
                        onClick={() => setView("boards")}
                      >
                        <ArrowLeft size={16} />
                        开发板列表
                      </Button>
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
                      <TextField
                        label="MAC 地址（可选）"
                        value={virtualMac}
                        onValue={setVirtualMac}
                        placeholder="留空自动生成"
                      />
                    </>
                  }
                  count={`${filteredVirtual.length} 台`}
                >
                  <TextField
                    label="搜索虚拟设备"
                    value={virtualSearch}
                    onValue={setVirtualSearch}
                    placeholder="名称、MAC 或 TAP"
                  />
                </ListToolbar>
              }
              footer={virtualPage.footer}
              scrollKey={virtualPage.viewKey}
            >
              {task.error && <Notice>{task.error}</Notice>}
              <DataTable
                headers={["设备", "MAC / TAP", "电源 / 串口", "操作"]}
                empty={!filteredVirtual.length}
              >
                {virtualPage.rows.map((d) => (
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
                        icon={ActionDelete}
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
            </PagePanel>
          </TabsContent>
        )}
      </Tabs>
    </>
  );
}
