import { Copy } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogBody,
  DialogFooter,
} from "@/components/ui/dialog";
import { MessageToast } from "@/components/message-toast";
import { toast } from "sonner";
import { usePermissions } from "@/components/permissions";
import { PagePanel, EditorFooter } from "@/components/page-panel";
import { useEffect, useId, useRef, useState } from "react";
import {
  Link,
  useNavigate,
  useParams,
  useSearchParams,
} from "react-router-dom";
import { api } from "@/api/client";
import {
  useList,
  useResource,
  useErrors,
  type PowerResult,
} from "@/api/events";
import {
  defaultFormState,
  boardToFormState,
  buildPowerManagementConfig,
  buildRequestPayload,
  validateForm,
  type BoardEditorFormState,
} from "@/apps/admin/board-form";
import type {
  BoardConfig,
  SerialPortSummary,
  SerialPortKeyKind,
  SerialPortKey,
} from "@/types/api";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Section,
  FieldTitle,
  TextField,
  ReloadNotice,
  FormGrid,
  TextareaField,
  SelectField,
  CheckField,
  Notice,
  ConfirmAction,
  useAction,
} from "@/components/forms";
import { MacPicker } from "@/apps/admin/components/mac-picker";
import { DtbUpload } from "./Dtbs";
import {
  SerialRuntimeDetails,
  serialRuntimeLabel,
} from "@/apps/admin/components/serial-runtime";
import { BoardOta } from "./BoardOta";
import { DeviceConnection } from "./DeviceConnection";

export default function BoardEditor() {
  const { boardId } = useParams();
  const data = useResource("board_editor");
  const error = useErrors().board_editor;
  if (boardId && error) return <MessageToast message={error} />;
  if (boardId && (!data || data.id !== boardId))
    return <Skeleton className="h-40 w-full" />;
  const board = boardId ? (data?.board ?? undefined) : undefined;
  if (boardId && !board)
    return (
      <Notice>
        开发板不存在或已被删除。<Link to="/boards">返回开发板</Link>
      </Notice>
    );
  return <Editor key={boardId ?? "new"} board={board} />;
}
function Editor({ board }: { board?: BoardConfig }) {
  const { local, can } = usePermissions();
  const canForceDelete =
    can("sessions.manage") && (local || can("rentals.manage"));
  const formId = useId();
  const [query, setQuery] = useSearchParams();
  const boards = useList("boards");
  const source = useResource("board_editor");
  const sourceError = useErrors().board_editor;
  const [copyOpen, setCopyOpen] = useState(false);
  const [selectedCopy, setSelectedCopy] = useState("");
  const copied = useRef<string | null>(null);
  const [copyRequest, setCopyRequest] = useState(0);
  const copyId = !board ? query.get("copy") : null;
  const navigate = useNavigate();
  const [form, setForm] = useState<BoardEditorFormState>(() => {
    const f = board ? boardToFormState(board) : defaultFormState();
    if (!board && query.get("mac")) {
      f.boot_kind = "httpboot";
      f.network_mac = query.get("mac")!;
    }
    if (!board && !local && f.boot_kind === "httpboot") f.disabled = true;
    return f;
  });
  useEffect(() => {
    if (
      !copyId ||
      source?.id !== copyId ||
      !source.board ||
      copied.current === copyId
    )
      return;
    const next = boardToFormState(source.board);
    next.id = "";
    if (query.get("mac")) next.network_mac = query.get("mac")!;
    if (!local && next.boot_kind === "httpboot") next.disabled = true;
    setForm(next);
    copied.current = copyId;
    toast.success("已复制开发板配置，请确认设备绑定后保存");
  }, [copyId, source, local, query, copyRequest]);
  const [baseline, setBaseline] = useState(() =>
    JSON.stringify(board ? boardToFormState(board) : null),
  );
  const [previousSource, setPreviousSource] = useState<string | null>(null);
  const save = useAction();
  const power = useAction();
  const serial = useList("serial"),
    dtbs = useList("dtbs"),
    loaders = useList("loaders"),
    virtual = useResource("virtual"),
    tftp = useResource("tftp_status"),
    actions = useList("power_actions"),
    sessions = useList("sessions"),
    serialManager = useResource("serial_manager");
  const storageKey = `ostool-power:${board?.id ?? "new"}`;
  const [actionId, setActionId] = useState(
    () => sessionStorage.getItem(storageKey) ?? "",
  );
  const [accepted, setAccepted] = useState<PowerResult>();
  const result = actions.find((a) => a.id === actionId) ?? accepted;
  const [requestedAction, setRequestedAction] = useState<"on" | "off">();
  const powerBusy = power.pending || result?.state === "running";
  const activeAction = power.pending
    ? requestedAction
    : result?.state === "running"
      ? result.action
      : undefined;
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
    setAccepted(undefined);
    setRequestedAction(action);
    await power.run(async () => {
      const value = await api.powerAction({
        request_id: id,
        action,
        power_management: buildPowerManagementConfig(form),
      });
      setAccepted(value);
    }, false);
    sending.current = false;
  }
  useEffect(() => {
    if (result && result.state !== "running")
      sessionStorage.removeItem(storageKey);
  }, [result, storageKey]);
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
  function portOwners(key: SerialPortKey) {
    if (!key.value) return [];
    const paths = matchingSerialPorts(serial, key).map(
      (p) => p.current_device_path,
    );
    return boards
      .filter(
        (b) =>
          !b.disabled &&
          b.id !== board?.id &&
          [
            ...(b.boot.kind !== "httpboot" && b.serial
              ? [{ key: b.serial.key, path: b.serial.resolved_device_path }]
              : []),
            ...(b.power_management.kind === "zhongsheng_relay"
              ? [
                  {
                    key: b.power_management.key,
                    path: b.power_management.resolved_device_path,
                  },
                ]
              : []),
          ].some(
            (port) =>
              (key.kind === port.key.kind && key.value === port.key.value) ||
              (port.path && paths.includes(port.path)) ||
              matchingSerialPorts(serial, port.key).some((p) =>
                paths.includes(p.current_device_path),
              ),
          ),
      )
      .map((b) => b.id);
  }
  const serialWarnings = [
    ...(form.boot_kind !== "httpboot" && form.serial_enabled
      ? [
          {
            label: "通信端口",
            key: { kind: form.serial_key_kind, value: form.serial_key_value },
          },
        ]
      : []),
    ...(form.power_management_kind === "zhongsheng_relay"
      ? [
          {
            label: "控制端口",
            key: {
              kind: form.relay_serial_key_kind,
              value: form.relay_serial_key_value,
            },
          },
        ]
      : []),
  ].flatMap(({ label, key }) => {
    const owners = portOwners(key);
    return owners.length
      ? [`${label}已被 ${owners.join("、")} 占用，请更换端口或禁用占用方。`]
      : [];
  });
  if (
    form.serial_enabled &&
    form.boot_kind !== "httpboot" &&
    form.power_management_kind === "zhongsheng_relay" &&
    form.serial_key_value &&
    form.relay_serial_key_value
  ) {
    const communication = {
      kind: form.serial_key_kind,
      value: form.serial_key_value,
    };
    const control = {
      kind: form.relay_serial_key_kind,
      value: form.relay_serial_key_value,
    };
    if (
      (communication.kind === control.kind &&
        communication.value === control.value) ||
      matchingSerialPorts(serial, communication).some((p) =>
        matchingSerialPorts(serial, control).some(
          (r) => r.current_device_path === p.current_device_path,
        ),
      )
    )
      serialWarnings.push(
        "控制端口已被当前配置的通信端口占用，请选择不同串口。",
      );
  }
  const serialOptions = (relay = false) => {
    const choices = [
      { value: "", label: "请选择稳定设备" },
      ...serial
        .filter((p) => !relay || p.stable_identity)
        .map((p) => ({
          value: `${p.primary_key_kind}:${p.primary_key_value}`,
          label: `${serialLabel(p)}${portOwners({
            kind: p.primary_key_kind!,
            value: p.primary_key_value!,
          })
            .map((id) => ` · 已被 ${id} 占用`)
            .join("")}`,
          disabled: !p.stable_identity,
        })),
      ...(!relay
        ? (virtual?.devices ?? []).map((d) => ({
            value: `qemu:${d.id}`,
            label: `QEMU · ${d.id} · ${d.mac_address}`,
          }))
        : []),
    ];
    const key: SerialPortKey = relay
      ? { kind: form.relay_serial_key_kind, value: form.relay_serial_key_value }
      : { kind: form.serial_key_kind, value: form.serial_key_value };
    const value = `${key.kind}:${key.value}`;
    if (!key.value || choices.some((choice) => choice.value === value))
      return choices;
    const matches = matchingSerialPorts(serial, key);
    if (matches.length > 0) {
      choices.push({
        value,
        label:
          matches.length === 1
            ? serialLabel(matches[0], key)
            : `${value}（匹配多个接口）`,
        disabled: matches.length > 1,
      });
    }
    return choices;
  };
  const text = (
    key: keyof BoardEditorFormState,
    label: string,
    props: { hint?: string; fullWidth?: boolean; disabled?: boolean } = {},
  ) => (
    <TextField
      key={key}
      label={label}
      value={String(form[key])}
      onValue={(v) => setForm((f) => ({ ...f, [key]: v }))}
      {...props}
    />
  );
  async function submit() {
    const error = validateForm(form);
    if (error) toast.error(error);
    if (error || conflict) return;
    await save.run(async () => {
      const payload = buildRequestPayload(form);
      const saved = board
        ? await api.updateBoard(board.id, payload)
        : await api.createBoard(payload);
      setPreviousSource(live);
      setForm((draft) => ({
        ...boardToFormState(saved),
        ...(saved.boot.kind === "httpboot"
          ? {
              serial_enabled: draft.serial_enabled,
              serial_key_kind: draft.serial_key_kind,
              serial_key_value: draft.serial_key_value,
              serial_baud_rate: draft.serial_baud_rate,
              axloader_serial_parameters_enabled:
                draft.axloader_serial_parameters_enabled,
              axloader_baud_rate: draft.axloader_baud_rate,
              axloader_data_bits: draft.axloader_data_bits,
              axloader_parity: draft.axloader_parity,
              axloader_stop_bits: draft.axloader_stop_bits,
              axloader_flow_control: draft.axloader_flow_control,
            }
          : {}),
      }));
      setBaseline(JSON.stringify(boardToFormState(saved)));
      navigate(`/boards/${encodeURIComponent(saved.id)}`);
    }, "已保存开发板");
  }
  const selectedLoader = loaders.find(
    (d) => d.mac_address === form.network_mac.trim().toLowerCase(),
  );
  const boundLoader = loaders.find(
    (d) => d.mac_address === board?.network_identity?.mac_address.toLowerCase(),
  );
  return (
    <PagePanel
      footer={
        <EditorFooter
          status={
            save.pending
              ? "正在保存…"
              : conflict
                ? "服务器配置已更新，草稿已保留"
                : JSON.stringify(form) !== baseline
                  ? "存在未保存的修改"
                  : ""
          }
        >
          {!board && (
            <Button
              type="button"
              variant="outline"
              disabled={save.pending || !boards.length}
              onClick={() => {
                setSelectedCopy("");
                setCopyOpen(true);
              }}
            >
              <Copy />
              复制已有数据
            </Button>
          )}
          {board && (
            <ConfirmAction
              label="删除开发板"
              description={
                canForceDelete
                  ? `删除 ${board.id}？将强制结束其会话${local ? "" : "并取消未结束租赁"}，不等待设备恢复；历史记录保留。`
                  : `删除 ${board.id} 的配置？`
              }
              action={async () => {
                await api.deleteBoard(board.id, canForceDelete);
                navigate("/boards");
              }}
            />
          )}
          <Button variant="outline" type="button" asChild>
            <Link to="/boards">取消</Link>
          </Button>
          <Button
            type="submit"
            form={formId}
            disabled={
              save.pending ||
              conflict ||
              (!!copyId && !sourceError && source?.id !== copyId)
            }
          >
            {save.pending ? "保存中…" : "保存开发板"}
          </Button>
        </EditorFooter>
      }
    >
      {conflict && (
        <ReloadNotice
          onReload={() => {
            setForm(boardToFormState(board!));
            setBaseline(live);
          }}
        >
          服务器配置已被其他操作修改。你的草稿已保留，请载入最新配置后重新编辑。
        </ReloadNotice>
      )}
      {!board && copyId && sourceError && (
        <MessageToast message={sourceError} />
      )}
      {!board && copyId && source?.id === copyId && !source.board && (
        <Notice>复制来源不存在或已被删除，请重新选择开发板。</Notice>
      )}
      <Dialog open={copyOpen} onOpenChange={setCopyOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>复制已有开发板数据</DialogTitle>
            <DialogDescription>
              复制会替换当前表单，ID 留空自动生成。串口、MAC
              和电源配置会一并复制，请确认它们属于新设备；设备认证需单独配置。
            </DialogDescription>
          </DialogHeader>
          <DialogBody>
            <SelectField
              label="已有开发板"
              value={selectedCopy}
              onValue={setSelectedCopy}
              options={[
                { value: "", label: "请选择开发板" },
                ...boards.map((b) => ({
                  value: b.id,
                  label: `${b.id} · ${b.board_type}`,
                })),
              ]}
            />
          </DialogBody>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setCopyOpen(false)}
            >
              取消
            </Button>
            <Button
              type="button"
              disabled={!boards.some((b) => b.id === selectedCopy)}
              onClick={() => {
                copied.current = null;
                setCopyRequest((value) => value + 1);
                setQuery((old) => {
                  const next = new URLSearchParams(old);
                  next.set("copy", selectedCopy);
                  return next;
                });
                setCopyOpen(false);
              }}
            >
              复制到当前表单
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      {serialWarnings.length > 0 && (
        <Notice>
          {serialWarnings.join(" ")}
          {form.disabled && " 当前为禁用草稿，可以保存；启用前需要解除冲突。"}
        </Notice>
      )}
      <form
        id={formId}
        className="board-editor-form"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <Section title="基本信息" columns={3}>
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
          <FormGrid columns={2} className="board-metadata-fields">
            <TextareaField
              fullWidth={false}
              label="标签"
              value={form.tags_text}
              onValue={(v) => set("tags_text", v)}
              hint="用逗号或换行分隔"
            />
            <TextareaField
              fullWidth={false}
              label="备注"
              value={form.notes}
              onValue={(v) => set("notes", v)}
            />
          </FormGrid>
          {!local && (
            <CheckField
              label="开放免申请"
              checked={form.allow_instant_use}
              onChange={(value) => set("allow_instant_use", value)}
              hint="仅拥有“免审批使用设备”权限的用户可直接领取；普通用户仍需申请审批，已有预约和使用限制继续生效。"
            />
          )}
          <CheckField
            label="禁用开发板"
            checked={form.disabled}
            onChange={(v) => set("disabled", v)}
          />
        </Section>
        <Section
          title="启动与绑定"
          columns={3}
          hint="选择启动方式后，下方对应配置可编辑；其他配置保留草稿。"
        >
          <SelectField
            label="启动方式"
            hint={
              !local
                ? "新增公网 HTTPboot 开发板会先禁用保存。保存后配置设备认证，再取消禁用并保存以开放分配。"
                : "启动方式决定下方配置是否可编辑。"
            }
            value={form.boot_kind}
            onValue={(v) =>
              setForm((f) => ({
                ...f,
                boot_kind: v as BoardEditorFormState["boot_kind"],
                disabled:
                  !board && !local && v === "httpboot" ? true : f.disabled,
              }))
            }
            options={[
              { value: "uboot", label: "U-Boot" },
              { value: "pxe", label: "PXE" },
              { value: "httpboot", label: "axloader (HTTP Boot)" },
            ]}
          />
          <MacPicker
            value={form.network_mac}
            onChange={(value) => set("network_mac", value)}
            devices={loaders}
            boardId={board?.id}
            disabled={form.boot_kind === "uboot"}
          />
          <TextField
            label="设备架构"
            readOnly
            value={
              selectedLoader?.online
                ? selectedLoader.arch
                : selectedLoader
                  ? "未知（设备离线）"
                  : "未知（未发现设备）"
            }
            hint={
              selectedLoader
                ? `${selectedLoader.ip_address} · ${selectedLoader.hardware.product} · ${selectedLoader.conflict ? "MAC 冲突" : selectedLoader.bound_board_id ? `已绑定 ${selectedLoader.bound_board_id}` : selectedLoader.online ? "在线，未绑定" : "离线，未绑定"}`
                : undefined
            }
          />
          <TextareaField
            label="PXE 备注"
            value={form.pxe_notes}
            disabled={form.boot_kind !== "pxe"}
            onValue={(v) => set("pxe_notes", v)}
          />
        </Section>
        <div className="board-editor-columns">
          <Section
            title="手动串口"
            columns={3}
            hint="用于 U-Boot 与 PXE；axloader HTTP Boot 使用下方自动串口。"
          >
            <CheckField
              label="启用串口"
              disabled={form.boot_kind === "httpboot"}
              checked={form.serial_enabled}
              onChange={(v) => set("serial_enabled", v)}
              hint="启用 U-Boot 开发板必须配置串口；未完成配置可先禁用保存。"
            />
            <SelectField
              label="稳定串口"
              disabled={form.boot_kind === "httpboot" || !form.serial_enabled}
              hint={serialDetails(serial, {
                kind: form.serial_key_kind,
                value: form.serial_key_value,
              })}
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
              disabled={form.boot_kind === "httpboot" || !form.serial_enabled}
              type="number"
              min={1}
              value={form.serial_baud_rate}
              onValue={(v) => set("serial_baud_rate", Number(v))}
            />
          </Section>
          <Section
            title="自动串口"
            columns={3}
            hint="仅用于 axloader HTTP Boot；未指定参数时采用设备上报值。"
          >
            <CheckField
              label="指定串口参数"
              disabled={form.boot_kind !== "httpboot"}
              checked={form.axloader_serial_parameters_enabled}
              onChange={(v) => set("axloader_serial_parameters_enabled", v)}
            />
            <div data-slot="field">
              <FieldTitle
                label="串口状态"
                hint={
                  <SerialRuntimeDetails
                    status={
                      sessions.find((session) => session.board_id === board?.id)
                        ?.serial_runtime
                    }
                  />
                }
              />
              <div className="board-readonly-value">
                {serialRuntimeLabel(
                  sessions.find((session) => session.board_id === board?.id)
                    ?.serial_runtime,
                )}
              </div>
            </div>
            <div data-slot="field">
              <FieldTitle label="自动发现" />
              <div className="board-readonly-value">
                发现请求 {serialManager?.pending ?? 0} · 候选监听{" "}
                {serialManager?.candidates ?? 0}
              </div>
            </div>
            <TextField
              disabled={
                form.boot_kind !== "httpboot" ||
                !form.axloader_serial_parameters_enabled
              }
              label="axloader 波特率"
              type="number"
              min={1}
              value={form.axloader_baud_rate}
              onValue={(v) => set("axloader_baud_rate", Number(v))}
              hint="整组参数优先于 axloader 上报；未配置时采用上报值或 115200/8N1"
            />
            <SelectField
              disabled={
                form.boot_kind !== "httpboot" ||
                !form.axloader_serial_parameters_enabled
              }
              label="数据位"
              value={String(form.axloader_data_bits)}
              onValue={(v) => set("axloader_data_bits", Number(v))}
              options={[
                { value: "7", label: "7" },
                { value: "8", label: "8" },
              ]}
            />
            <SelectField
              disabled={
                form.boot_kind !== "httpboot" ||
                !form.axloader_serial_parameters_enabled
              }
              label="校验"
              value={form.axloader_parity}
              onValue={(v) =>
                set(
                  "axloader_parity",
                  v as BoardEditorFormState["axloader_parity"],
                )
              }
              options={[
                { value: "none", label: "无" },
                { value: "odd", label: "奇校验" },
                { value: "even", label: "偶校验" },
              ]}
            />
            <SelectField
              disabled={
                form.boot_kind !== "httpboot" ||
                !form.axloader_serial_parameters_enabled
              }
              label="停止位"
              value={form.axloader_stop_bits}
              onValue={(v) =>
                set(
                  "axloader_stop_bits",
                  v as BoardEditorFormState["axloader_stop_bits"],
                )
              }
              options={[
                { value: "one", label: "1" },
                { value: "two", label: "2" },
              ]}
            />
            <SelectField
              disabled={
                form.boot_kind !== "httpboot" ||
                !form.axloader_serial_parameters_enabled
              }
              label="流控"
              value={form.axloader_flow_control}
              onValue={(v) =>
                set(
                  "axloader_flow_control",
                  v as BoardEditorFormState["axloader_flow_control"],
                )
              }
              options={[
                { value: "none", label: "无" },
                { value: "rts_cts", label: "RTS/CTS" },
              ]}
            />
          </Section>
        </div>
        <div className="board-editor-columns">
          <div>
            <Section
              title="U-Boot 启动"
              columns={3}
              hint="仅 U-Boot 启动使用这些配置；加载地址留空时使用默认值。"
            >
              <div className="board-inline-field" data-slot="field">
                <SelectField
                  label="预设 DTB"
                  disabled={form.boot_kind !== "uboot"}
                  value={form.dtb_name}
                  onValue={(v) => set("dtb_name", v)}
                  options={[
                    { value: "", label: "无预设" },
                    ...dtbs.map((d) => ({
                      value: d.name,
                      label: d.disabled ? `${d.name}（已禁用）` : d.name,
                      disabled: d.disabled,
                    })),
                  ]}
                />
                <DtbUpload
                  disabled={form.boot_kind !== "uboot"}
                  onUploaded={(name) => set("dtb_name", name)}
                />
              </div>
              {text("kernel_load_addr", "Kernel 加载地址", {
                disabled: form.boot_kind !== "uboot",
              })}
              {text("fit_load_addr", "FIT 加载地址", {
                disabled: form.boot_kind !== "uboot",
              })}
              {text("bootm_addr", "bootm 地址", {
                disabled: form.boot_kind !== "uboot",
              })}
              <CheckField
                label="启用 TFTP"
                disabled={form.boot_kind !== "uboot"}
                checked={form.use_tftp}
                onChange={(v) => set("use_tftp", v)}
              />
            </Section>
          </div>
          <div>
            <Section
              title="TFTP 网络"
              columns={3}
              hint="启用 U-Boot TFTP 后可编辑；静态网络参数仅在静态 IP 模式下生效。"
            >
              <SelectField
                label="网络模式"
                disabled={form.boot_kind !== "uboot" || !form.use_tftp}
                value={form.network_mode}
                onValue={(v) => set("network_mode", v as "dhcp" | "static_ip")}
                options={[
                  { value: "dhcp", label: "DHCP" },
                  { value: "static_ip", label: "静态 IP" },
                ]}
              />
              {text("board_ip", "开发板 IP", {
                disabled:
                  form.boot_kind !== "uboot" ||
                  !form.use_tftp ||
                  form.network_mode !== "static_ip",
              })}
              {text("server_ip", "服务器 IP", {
                disabled:
                  form.boot_kind !== "uboot" ||
                  !form.use_tftp ||
                  form.network_mode !== "static_ip",
                hint: `默认 ${tftp?.resolved_server_ip ?? "未解析"}`,
              })}
              {text("netmask", "子网掩码", {
                disabled:
                  form.boot_kind !== "uboot" ||
                  !form.use_tftp ||
                  form.network_mode !== "static_ip",
                hint: `默认 ${tftp?.resolved_netmask ?? "未解析"}`,
              })}
              {text("gatewayip", "网关", {
                disabled:
                  form.boot_kind !== "uboot" ||
                  !form.use_tftp ||
                  form.network_mode !== "static_ip",
              })}
            </Section>
          </div>
        </div>
        <Section
          title="电源管理"
          columns={3}
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
          <SelectField
            disabled={form.power_management_kind !== "qemu"}
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
          <SelectField
            disabled={form.power_management_kind !== "zhongsheng_relay"}
            label="继电器"
            hint={serialDetails(serial, {
              kind: form.relay_serial_key_kind,
              value: form.relay_serial_key_value,
            })}
            value={
              form.relay_serial_key_value
                ? `${form.relay_serial_key_kind}:${form.relay_serial_key_value}`
                : ""
            }
            onValue={(v) => chooseSerial(v, true)}
            options={serialOptions(true)}
          />
          <FormGrid columns={2}>
            <div
              className="board-inline-field"
              data-slot="field"
              role="group"
              aria-label="上电测试"
            >
              <TextField
                label="上电命令"
                disabled={form.power_management_kind !== "custom"}
                value={form.power_on_cmd}
                onValue={(value) => set("power_on_cmd", value)}
              />
              <Button
                permission="devices.power"
                type="button"
                variant="outline"
                disabled={powerBusy}
                onClick={() => void sendPower("on")}
              >
                {activeAction === "on" ? "测试中…" : "上电测试"}
              </Button>
            </div>
            <div
              className="board-inline-field"
              data-slot="field"
              role="group"
              aria-label="下电测试"
            >
              <TextField
                label="下电命令"
                disabled={form.power_management_kind !== "custom"}
                value={form.power_off_cmd}
                onValue={(value) => set("power_off_cmd", value)}
              />
              <Button
                permission="devices.power"
                type="button"
                variant="outline"
                disabled={powerBusy}
                onClick={() => void sendPower("off")}
              >
                {activeAction === "off" ? "测试中…" : "下电测试"}
              </Button>
            </div>
          </FormGrid>
        </Section>
        <MessageToast
          kind={result?.state === "failed" ? "error" : "success"}
          message={
            result?.state === "succeeded"
              ? `${result.action === "on" ? "上电" : "下电"}命令已完成`
              : result?.state === "failed"
                ? `电源命令失败：${result.message}`
                : null
          }
        />
        <Section title="设备恢复">
          <TextField
            label="释放后恢复命令"
            fullWidth
            value={form.reset_command}
            onValue={(v) => setForm((f) => ({ ...f, reset_command: v }))}
            hint="在断电后执行的幂等恢复命令。纯 RAM 启动可留空。"
            placeholder="可选"
          />
        </Section>
      </form>
      {board?.network_identity && board.boot.kind === "httpboot" && (
        <>
          <DeviceConnection boardId={board.id} />
          <BoardOta boardId={board.id} />
        </>
      )}
    </PagePanel>
  );
}
function matchingSerialPorts(ports: SerialPortSummary[], key: SerialPortKey) {
  const { kind, value } = key;
  if (!value || kind === "qemu") return [];
  return ports.filter((port) =>
    kind === "serial_number"
      ? port.serial_number === value
      : port.usb_id === value || port.usb_path === value,
  );
}
function serialLabel(p: SerialPortSummary, key?: SerialPortKey) {
  const keyKind = key?.kind ?? p.primary_key_kind;
  const kind =
    keyKind === "serial_number"
      ? "SN"
      : keyKind === "usb_id"
        ? "USB ID"
        : "USB PATH";
  return `${kind} ${key?.value ?? p.primary_key_value ?? "无稳定标识"} · ${p.current_device_path} · ${p.manufacturer ?? ""} ${p.product ?? ""}`;
}
function serialDetails(
  ports: SerialPortSummary[],
  portKey: SerialPortKey,
): string | undefined {
  if (!portKey.value) return undefined;
  const matches = matchingSerialPorts(ports, portKey);
  if (matches.length > 1)
    return "匹配多个串口接口，请选择 USB ID 或 USB PATH。";
  const port = matches[0];
  return port
    ? `${port.usb_id ?? port.usb_path ?? ""} · ${port.current_device_path} · ${port.manufacturer ?? ""} ${port.product ?? ""} · VID:PID ${port.usb_vendor_id?.toString(16) ?? "—"}:${port.usb_product_id?.toString(16) ?? "—"}`
    : "当前未检测到该设备，保留稳定标识";
}
