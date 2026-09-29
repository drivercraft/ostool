import { useEffect, useRef, useState } from "react";
import {
  Link,
  useNavigate,
  useParams,
  useSearchParams,
} from "react-router-dom";
import { api } from "@/api/client";
import { useList, useResource, type PowerResult } from "@/api/events";
import {
  defaultFormState,
  boardToFormState,
  buildPowerManagementConfig,
  buildRequestPayload,
  validateForm,
  type BoardEditorFormState,
} from "@/board-form";
import type {
  BoardConfig,
  SerialPortSummary,
  SerialPortKeyKind,
} from "@/types/api";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Section,
  TextField,
  TextareaField,
  SelectField,
  CheckField,
  Notice,
  ConfirmAction,
  useAction,
  options,
} from "@/components/forms";
import { MacPicker } from "@/components/mac-picker";
import { DtbUpload } from "./Dtbs";
import { BoardOta } from "./BoardOta";

export default function BoardEditor() {
  const { boardId } = useParams();
  const boards = useResource("boards");
  if (boardId && !boards) return <Skeleton className="h-40 w-full" />;
  const board = boards?.find((b) => b.id === boardId);
  if (boardId && !board)
    return (
      <Notice>
        开发板不存在或已被删除。<Link to="/boards">返回开发板</Link>
      </Notice>
    );
  return <Editor key={boardId ?? "new"} board={board} />;
}
function Editor({ board }: { board?: BoardConfig }) {
  const [query] = useSearchParams();
  const navigate = useNavigate();
  const [form, setForm] = useState<BoardEditorFormState>(() => {
    const f = board ? boardToFormState(board) : defaultFormState();
    if (!board && query.get("mac")) {
      f.boot_kind = "httpboot";
      f.network_mac = query.get("mac")!;
    }
    return f;
  });
  const [baseline, setBaseline] = useState(() =>
    JSON.stringify(board ? boardToFormState(board) : null),
  );
  const [previousSource, setPreviousSource] = useState<string | null>(null);
  const [validation, setValidation] = useState("");
  const save = useAction();
  const power = useAction();
  const serial = useList("serial"),
    dtbs = useList("dtbs"),
    loaders = useList("loaders"),
    virtual = useResource("virtual"),
    tftp = useResource("tftp_status"),
    actions = useList("power_actions");
  const storageKey = `ostool-power:${board?.id ?? "new"}`;
  const [actionId, setActionId] = useState(
    () => sessionStorage.getItem(storageKey) ?? "",
  );
  const [accepted, setAccepted] = useState<PowerResult>();
  const result = actions.find((a) => a.id === actionId) ?? accepted;
  const [uncertain, setUncertain] = useState(false);
  const powerBusy = power.pending || result?.state === "running" || uncertain;
  const sending = useRef(false);
  const live = JSON.stringify(board ? boardToFormState(board) : null);
  const conflict = !!board && live !== baseline && live !== previousSource;
  useEffect(() => {
    if (live === baseline) setPreviousSource(null);
  }, [live, baseline]);
  function set<K extends keyof BoardEditorFormState>(
    key: K,
    value: BoardEditorFormState[K],
  ) {
    setForm((f) => ({ ...f, [key]: value }));
  }
  async function sendPower(action: "on" | "off") {
    if (sending.current || powerBusy) return;
    sending.current = true;
    const id = crypto.randomUUID();
    setActionId(id);
    sessionStorage.setItem(storageKey, id);
    setUncertain(true);
    await power.run(async () => {
      const value = await api.powerAction({
        request_id: id,
        action,
        power_management: buildPowerManagementConfig(form),
      });
      setAccepted(value);
      setUncertain(false);
    });
    sending.current = false;
  }
  useEffect(() => {
    if (actions.some((a) => a.id === actionId)) setUncertain(false);
  }, [actions, actionId]);
  function chooseSerial(value: string, relay = false) {
    const colon = value.indexOf(":");
    const kind = value.slice(0, colon) as SerialPortKeyKind;
    const key = value.slice(colon + 1);
    setForm((f) =>
      relay
        ? { ...f, relay_serial_key_kind: kind, relay_serial_key_value: key }
        : { ...f, serial_key_kind: kind, serial_key_value: key },
    );
  }
  const serialOptions = (relay = false) => [
    { value: "", label: "请选择稳定设备" },
    ...serial
      .filter((p) => !relay || p.stable_identity)
      .map((p) => ({
        value: `${p.primary_key_kind}:${p.primary_key_value}`,
        label: serialLabel(p),
        disabled: !p.stable_identity,
      })),
    ...(!relay
      ? (virtual?.devices ?? []).map((d) => ({
          value: `qemu:${d.id}`,
          label: `QEMU · ${d.id} · ${d.mac_address}`,
        }))
      : []),
  ];
  const text = (
    key: keyof BoardEditorFormState,
    label: string,
    hint?: string,
  ) => (
    <TextField
      key={key}
      label={label}
      value={String(form[key])}
      onValue={(v) => setForm((f) => ({ ...f, [key]: v }))}
      hint={hint}
    />
  );
  async function submit() {
    const error = validateForm(form);
    setValidation(error);
    if (error || conflict || powerBusy) return;
    await save.run(async () => {
      const payload = buildRequestPayload(form);
      const saved = board
        ? await api.updateBoard(board.id, payload)
        : await api.createBoard(payload);
      setPreviousSource(live);
      setForm(boardToFormState(saved));
      setBaseline(JSON.stringify(boardToFormState(saved)));
      navigate(`/boards/${encodeURIComponent(saved.id)}`);
    }, "已保存开发板");
  }
  const selectedLoader = loaders.find(
    (d) => d.mac_address === form.network_mac.trim().toLowerCase(),
  );
  return (
    <>
      <header className="page-header">
        <div>
          <Link className="back-link" to="/boards">
            开发板 /
          </Link>
          <h1>{board ? "编辑开发板" : "新建开发板"}</h1>
        </div>
        {board && (
          <ConfirmAction
            label="删除开发板"
            description={`删除 ${board.id} 的配置？`}
            disabled={powerBusy}
            action={async () => {
              await api.deleteBoard(board.id);
              navigate("/boards");
            }}
          />
        )}
      </header>
      {conflict && (
        <Notice>
          服务器配置已被其他操作修改。你的草稿已保留，请载入最新配置后重新编辑。
          <Button
            variant="outline"
            onClick={() => {
              setForm(boardToFormState(board!));
              setBaseline(live);
            }}
          >
            载入服务器版本
          </Button>
        </Notice>
      )}
      {(validation || save.error) && (
        <Notice>{validation || save.error}</Notice>
      )}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <Section title="基本信息">
          <TextField
            label="板型"
            required
            value={form.board_type}
            onValue={(v) => set("board_type", v)}
            hint="用于资源分配的板型标签"
          />
          <TextField
            label="ID"
            value={form.id}
            onValue={(v) => set("id", v)}
            hint="留空自动生成"
          />
          <TextareaField
            label="标签"
            value={form.tags_text}
            onValue={(v) => set("tags_text", v)}
            hint="用逗号或换行分隔"
          />
          <TextareaField
            label="备注"
            value={form.notes}
            onValue={(v) => set("notes", v)}
          />
          <CheckField
            label="禁用开发板"
            checked={form.disabled}
            onChange={(v) => set("disabled", v)}
          />
        </Section>
        <Section title="串口">
          <CheckField
            label="启用串口"
            checked={form.serial_enabled}
            onChange={(v) => set("serial_enabled", v)}
          />
          {form.serial_enabled && (
            <>
              <SelectField
                label="稳定串口"
                value={
                  form.serial_key_value
                    ? `${form.serial_key_kind}:${form.serial_key_value}`
                    : ""
                }
                onValue={(v) => chooseSerial(v)}
                options={serialOptions()}
              />
              <TextField
                label="波特率"
                type="number"
                min={1}
                value={form.serial_baud_rate}
                onValue={(v) => set("serial_baud_rate", Number(v))}
              />
              <SerialDetails ports={serial} value={form.serial_key_value} />
            </>
          )}
        </Section>
        <fieldset disabled={powerBusy} className="power-fields">
          <Section
            title="电源管理"
            hint="选择模块后可直接上电或下电，无需先保存开发板。"
          >
            <SelectField
              label="电源模块"
              value={form.power_management_kind}
              onValue={(v) =>
                set(
                  "power_management_kind",
                  v as BoardEditorFormState["power_management_kind"],
                )
              }
              options={[
                { value: "custom", label: "Custom 命令" },
                { value: "zhongsheng_relay", label: "中盛继电器" },
                { value: "qemu", label: "QEMU 虚拟设备" },
              ]}
            />
            {form.power_management_kind === "custom" && (
              <>
                {text("power_on_cmd", "上电命令")}
                {text("power_off_cmd", "下电命令")}
              </>
            )}
            {form.power_management_kind === "zhongsheng_relay" && (
              <>
                <SelectField
                  label="继电器"
                  value={
                    form.relay_serial_key_value
                      ? `${form.relay_serial_key_kind}:${form.relay_serial_key_value}`
                      : ""
                  }
                  onValue={(v) => chooseSerial(v, true)}
                  options={serialOptions(true)}
                />
                <SerialDetails
                  ports={serial}
                  value={form.relay_serial_key_value}
                />
              </>
            )}
            {form.power_management_kind === "qemu" && (
              <SelectField
                label="虚拟设备"
                value={form.virtual_device_id}
                onValue={(v) => set("virtual_device_id", v)}
                options={[
                  { value: "", label: "选择已有虚拟设备" },
                  ...(virtual?.devices ?? []).map((d) => ({
                    value: d.id,
                    label: `${d.id} · ${d.mac_address}`,
                  })),
                ]}
              />
            )}
            <div className="actions">
              <Button type="button" onClick={() => void sendPower("on")}>
                上电
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={() => void sendPower("off")}
              >
                下电
              </Button>
            </div>
          </Section>
        </fieldset>
        <div className="power-feedback" aria-live="polite">
          {result?.state === "running"
            ? "电源命令执行中…"
            : result?.state === "succeeded"
              ? `${result.action === "on" ? "上电" : "下电"}命令已完成`
              : result?.state === "failed"
                ? `电源命令失败：${result.message}`
                : null}
          {(uncertain || (actionId && !result)) && (
            <Notice>
              动作结果尚未确认。查询已有动作不会重复执行命令。
              <Button
                type="button"
                variant="outline"
                disabled={power.pending}
                onClick={() =>
                  void power.run(async () => {
                    setAccepted(await api.getPowerAction(actionId));
                    setUncertain(false);
                  })
                }
              >
                查询执行结果
              </Button>
            </Notice>
          )}
          {power.error && (
            <Notice>
              {power.error}
              <Button
                type="button"
                variant="outline"
                onClick={() => {
                  setUncertain(false);
                  setAccepted(undefined);
                  setActionId("");
                  sessionStorage.removeItem(storageKey);
                }}
              >
                清除结果提示
              </Button>
            </Notice>
          )}
        </div>
        <Section title="启动与绑定">
          <SelectField
            label="启动方式"
            value={form.boot_kind}
            onValue={(v) =>
              set("boot_kind", v as BoardEditorFormState["boot_kind"])
            }
            options={[
              { value: "uboot", label: "U-Boot" },
              { value: "pxe", label: "PXE" },
              { value: "httpboot", label: "HTTP Boot" },
            ]}
          />
          {form.boot_kind !== "uboot" && (
            <MacPicker
              value={form.network_mac}
              onChange={(v) => set("network_mac", v)}
              devices={loaders}
              boardId={board?.id}
            />
          )}
          {form.boot_kind !== "uboot" && selectedLoader && (
            <p className="hint">
              {selectedLoader.ip_address} · 设备架构 {selectedLoader.arch} ·{" "}
              {selectedLoader.hardware.product} ·{" "}
              {selectedLoader.conflict
                ? "MAC 冲突"
                : selectedLoader.bound_board_id
                  ? `已绑定 ${selectedLoader.bound_board_id}`
                  : selectedLoader.online
                    ? "在线，未绑定"
                    : "离线，未绑定"}
            </p>
          )}
          {form.boot_kind === "httpboot" && (
            <details className="advanced-options full-width">
              <summary>
                高级启动设置 <span>默认使用 x86_64，通常无需修改</span>
              </summary>
              <div className="advanced-fields">
                <SelectField
                  label="启动架构"
                  value={form.boot_arch}
                  onValue={(v) => set("boot_arch", v)}
                  options={[
                    { value: "", label: "默认（x86_64）" },
                    ...options([
                      "x86_64",
                      "aarch64",
                      "loongarch64",
                      "riscv64",
                      "other",
                    ]),
                  ]}
                />
              </div>
            </details>
          )}
          {form.boot_kind === "pxe" && (
            <TextareaField
              label="PXE 备注"
              value={form.pxe_notes}
              onValue={(v) => set("pxe_notes", v)}
            />
          )}
          {form.boot_kind === "uboot" && (
            <>
              <CheckField
                label="启用 TFTP"
                checked={form.use_tftp}
                onChange={(v) => set("use_tftp", v)}
              />
              <SelectField
                label="预设 DTB"
                value={form.dtb_name}
                onValue={(v) => set("dtb_name", v)}
                options={[
                  { value: "", label: "无预设" },
                  ...dtbs.map((d) => ({ value: d.name, label: d.name })),
                ]}
              />
              <DtbUpload onUploaded={(name) => set("dtb_name", name)} />
              <details
                className="advanced-options full-width"
                open={
                  !!(
                    form.kernel_load_addr ||
                    form.fit_load_addr ||
                    form.bootm_addr
                  )
                }
              >
                <summary>
                  高级启动地址 <span>按需配置，留空使用默认值</span>
                </summary>
                <div className="advanced-fields">
                  {text("kernel_load_addr", "Kernel 加载地址")}
                  {text("fit_load_addr", "FIT 加载地址")}
                  {text("bootm_addr", "bootm 地址")}
                </div>
              </details>
              {form.use_tftp && (
                <>
                  <SelectField
                    label="网络模式"
                    value={form.network_mode}
                    onValue={(v) =>
                      set("network_mode", v as "dhcp" | "static_ip")
                    }
                    options={[
                      { value: "dhcp", label: "DHCP" },
                      { value: "static_ip", label: "静态 IP" },
                    ]}
                  />
                  {form.network_mode === "static_ip" && (
                    <>
                      {text("board_ip", "开发板 IP")}
                      {text(
                        "server_ip",
                        "服务器 IP",
                        `默认 ${tftp?.resolved_server_ip ?? "未解析"}`,
                      )}
                      {text(
                        "netmask",
                        "子网掩码",
                        `默认 ${tftp?.resolved_netmask ?? "未解析"}`,
                      )}
                      {text("gatewayip", "网关")}
                    </>
                  )}
                </>
              )}
            </>
          )}
        </Section>
        <footer className="form-footer">
          <Button variant="outline" type="button" asChild>
            <Link to="/boards">返回</Link>
          </Button>
          <Button
            type="submit"
            disabled={save.pending || powerBusy || conflict}
          >
            {save.pending ? "保存中…" : "保存开发板"}
          </Button>
        </footer>
      </form>
      {board?.network_identity &&
        board.boot.kind === "httpboot" &&
        (!board.boot.boot_arch || board.boot.boot_arch === "x86_64") && (
          <BoardOta boardId={board.id} />
        )}
    </>
  );
}
function serialLabel(p: SerialPortSummary) {
  return `${p.primary_key_kind === "serial_number" ? "SN" : "USB PATH"} ${p.primary_key_value ?? "无稳定标识"} · ${p.current_device_path} · ${p.manufacturer ?? ""} ${p.product ?? ""}`;
}
function SerialDetails({
  ports,
  value,
}: {
  ports: SerialPortSummary[];
  value: string;
}) {
  const p = ports.find((p) => p.primary_key_value === value);
  if (!value) return null;
  return (
    <p className="hint full-width">
      {p
        ? `${p.usb_path ?? ""} · ${p.current_device_path} · ${p.manufacturer ?? ""} ${p.product ?? ""} · VID:PID ${p.usb_vendor_id?.toString(16) ?? "—"}:${p.usb_product_id?.toString(16) ?? "—"}`
        : "当前未检测到该设备，保留稳定标识"}
    </p>
  );
}
