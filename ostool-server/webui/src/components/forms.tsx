import {
  createContext,
  useContext,
  useId,
  useRef,
  useState,
  Children,
  isValidElement,
  type ReactNode,
  type ComponentProps,
} from "react";
import {
  ListLayoutProvider,
  ListLayoutToggle,
  useListLayout,
} from "./list-layout";
import { toast } from "sonner";
import { Inbox, type LucideIcon } from "lucide-react";
import { RowAction } from "./row-action";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Skeleton } from "@/components/ui/skeleton";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Field,
  FieldGroup,
  FieldLabel,
  FieldDescription,
} from "@/components/ui/field";
import { Alert, AlertDescription } from "@/components/ui/alert";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import {
  Table,
  TableHeader,
  TableRow,
  TableHead,
  TableBody,
  TableCell,
} from "@/components/ui/table";
import {
  Empty,
  EmptyHeader,
  EmptyTitle,
  EmptyMedia,
} from "@/components/ui/empty";

const inputHints: Record<string, string> = {
  ID: "开发板的唯一标识；新增时留空可自动生成，编辑后会影响按 ID 访问。",
  配置文件: "服务器上的绝对路径，例如 /etc/default/tftpd-hpa。",
  服务名: "由 systemd 管理的 TFTP 服务名称，例如 tftpd-hpa。",
  运行用户: "运行该服务的系统用户，例如 tftp。",
  监听地址: "IP:端口；仅监听端口可使用 :69，具体格式遵循所选服务。",
  启动选项:
    "传递给 TFTP 服务的参数；使用只读映射 -l -s --map-file /etc/ostool-server-tftp.rules。",
  上电命令: "在服务器执行的电源开启命令；仅填写受信任的管理员命令。",
  下电命令: "在服务器执行的电源关闭命令；会话释放时可能自动执行。",
  新密码: "至少 15 个字符，最多 1024 字节；建议使用随机长密码。",
  当前密码: "验证当前登录账号的密码；此内容不会显示在审计日志中。",
  使用用户: "选择已启用用户；租赁生效后该用户可在指定时段使用设备。",
  板型: "例如 rk3568、qemu-aarch64；CLI 按板型选择可用设备。",
  标签: "多个标签使用英文逗号分隔，用于筛选和分类。",
  备注: "选填；仅记录用途和说明，不要填写密码、令牌等凭据。",
  波特率: "串口每秒位数，例如 115200；应与设备固件配置一致。",
  根目录: "服务器上的绝对目录路径，例如 /srv/ostool/tftp。",
  绑定地址: "IP:端口，例如 0.0.0.0:69；IPv6 使用 [::]:69。",
  网络接口: "选择开发板所在网卡，服务器通过该接口解析设备网络地址。",
  "Session 文件上传上限（MiB）":
    "单个会话文件的大小上限，单位 MiB，必须为大于零的整数。",
  邮箱: "请输入可接收邮件的完整邮箱地址，例如 name@example.com。",
  显示名称: "用于页面和操作记录中的展示，不作为登录凭据。",
  角色名称: "使用便于识别的名称；名称本身不会授予额外权限。",
  指定开发板: "仅可选择已启用设备；日历灰色时段表示已占用或已过期。",
};

export function TextField({
  label,
  hint,
  onValue,
  inlineHint = false,
  ...props
}: ComponentProps<typeof Input> & {
  label: string;
  hint?: string;
  inlineHint?: boolean;
  onValue?: (value: string) => void;
}) {
  const id = useId();
  const description = hint ?? inputHints[label];
  return (
    <Field>
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
      <Input
        id={id}
        {...props}
        placeholder={
          props.placeholder ?? (inlineHint ? description : undefined)
        }
        aria-describedby={
          description && !inlineHint ? `${id}-hint` : props["aria-describedby"]
        }
        onChange={onValue ? (e) => onValue(e.target.value) : props.onChange}
      />
      {description && !inlineHint && (
        <FieldDescription id={`${id}-hint`}>{description}</FieldDescription>
      )}
    </Field>
  );
}
export function TextareaField({
  label,
  hint,
  onValue,
  ...props
}: Omit<ComponentProps<typeof Textarea>, "onChange"> & {
  label: string;
  hint?: string;
  onValue: (value: string) => void;
}) {
  const id = useId();
  const description = hint ?? inputHints[label];
  return (
    <Field>
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
      <Textarea
        id={id}
        rows={2}
        {...props}
        aria-describedby={
          description ? `${id}-hint` : props["aria-describedby"]
        }
        onChange={(e) => onValue(e.target.value)}
      />
      {description && (
        <FieldDescription id={`${id}-hint`}>{description}</FieldDescription>
      )}
    </Field>
  );
}
export function SelectField({
  label,
  hint,
  options,
  onValue,
  ...props
}: Omit<ComponentProps<typeof NativeSelect>, "onChange"> & {
  label: string;
  hint?: string;
  options: { value: string; label: string; disabled?: boolean }[];
  onValue: (value: string) => void;
}) {
  const id = useId();
  const description = hint ?? inputHints[label];
  const optionsWithCurrent = [...options];
  if (props.value && !options.some((o) => o.value === props.value))
    optionsWithCurrent.unshift({
      value: String(props.value),
      label: `${props.value}（当前未检测到）`,
    });
  return (
    <Field>
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
      <NativeSelect
        id={id}
        className="w-full"
        {...props}
        aria-describedby={
          description ? `${id}-hint` : props["aria-describedby"]
        }
        onChange={(e) => onValue(e.target.value)}
      >
        {optionsWithCurrent.map((o) => (
          <option key={o.value} value={o.value} disabled={o.disabled}>
            {o.label}
          </option>
        ))}
      </NativeSelect>
      {description && (
        <FieldDescription id={`${id}-hint`}>{description}</FieldDescription>
      )}
    </Field>
  );
}
export function CheckField({
  label,
  hint,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  checked: boolean;
  hint?: string;
  onChange: (value: boolean) => void;
  disabled?: boolean;
}) {
  const id = useId();
  return (
    <Field orientation="horizontal">
      <Checkbox
        id={id}
        checked={checked}
        aria-describedby={hint ? `${id}-hint` : undefined}
        onCheckedChange={(value) => onChange(value === true)}
        disabled={disabled}
      />
      <div>
        <FieldLabel htmlFor={id}>{label}</FieldLabel>
        {hint && <FieldDescription id={`${id}-hint`}>{hint}</FieldDescription>}
      </div>
    </Field>
  );
}
export function Section({
  title,
  hint,
  children,
}: {
  title: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <section className="form-section">
      <div>
        <h2>{title}</h2>
        {hint && <p className="hint">{hint}</p>}
      </div>
      <FieldGroup className="form-fields">{children}</FieldGroup>
    </section>
  );
}
export function Notice({ children }: { children: ReactNode }) {
  return (
    <Alert>
      <AlertDescription>{children}</AlertDescription>
    </Alert>
  );
}
export function useAction() {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const running = useRef(false);
  async function run(action: () => Promise<unknown>, message?: string) {
    if (running.current) return false;
    running.current = true;
    setPending(true);
    setError("");
    try {
      await action();
      if (message) toast.success(message);
      return true;
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setError(message);
      toast.error(message);
      return false;
    } finally {
      running.current = false;
      setPending(false);
    }
  }
  return { pending, error, run };
}
export function ConfirmAction({
  label,
  description,
  action,
  disabled,
  triggerVariant = "destructive",
  icon,
}: {
  label: string;
  description: string;
  action: () => Promise<unknown>;
  disabled?: boolean;
  triggerVariant?: "ghost" | "destructive";
  icon?: LucideIcon;
}) {
  const [open, setOpen] = useState(false);
  const task = useAction();
  return (
    <>
      {icon ? (
        <RowAction
          label={label}
          icon={icon}
          variant={triggerVariant}
          disabled={disabled}
          onClick={() => setOpen(true)}
        />
      ) : (
        <Button
          type="button"
          variant={triggerVariant}
          size="sm"
          disabled={disabled}
          onClick={() => setOpen(true)}
        >
          {label}
        </Button>
      )}
      <Dialog
        open={open}
        onOpenChange={(v) => {
          if (!task.pending) setOpen(v);
        }}
      >
        <DialogContent>
          <DialogTitle>{label}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
          {task.error && <Notice>{task.error}</Notice>}
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setOpen(false)}
              disabled={task.pending}
            >
              取消
            </Button>
            <Button
              variant="destructive"
              disabled={task.pending}
              onClick={() =>
                void task.run(action, "操作已提交").then((ok) => {
                  if (ok) setOpen(false);
                })
              }
            >
              {task.pending ? "提交中…" : "确认"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
export const ListDataLoadingContext = createContext(false);

export function DataTable({
  headers,
  children,
  empty,
  layout = "switchable",
}: {
  headers: string[];
  children: ReactNode;
  empty?: boolean;
  layout?: "switchable" | "table";
}) {
  const view = useListLayout();
  const loading = useContext(ListDataLoadingContext);
  if (layout === "switchable" && !view)
    return (
      <ListLayoutProvider
        storageKey={`${window.location.pathname}:${headers.join("/")}`}
      >
        <div className="standalone-layout-tools">
          <ListLayoutToggle />
        </div>
        <DataTable headers={headers} empty={empty}>
          {children}
        </DataTable>
      </ListLayoutProvider>
    );
  if (layout === "switchable" && view?.layout === "cards" && loading)
    return (
      <div className="data-card-grid" role="status" aria-label="正在加载数据">
        {[0, 1, 2].map((index) => (
          <div className="data-card" key={index}>
            <Skeleton className="h-5 w-2/3" />
            <Skeleton className="mt-6 h-20 w-full" />
          </div>
        ))}
      </div>
    );
  if (layout === "switchable" && view?.layout === "cards" && empty)
    return <ListEmpty />;
  if (layout === "switchable" && view?.layout === "cards")
    return (
      <div className="data-card-grid" role="list" aria-label="卡片列表">
        {Children.toArray(children).map((row, rowIndex) => {
          if (!isValidElement<{ children?: ReactNode }>(row)) return null;
          const cells = Children.toArray(row.props.children).filter(
            isValidElement<{ children?: ReactNode; className?: string }>,
          );
          return (
            <article
              className="data-card"
              role="listitem"
              key={row.key ?? rowIndex}
            >
              <div
                className={`data-card-title ${cells[0]?.props.className ?? ""}`}
              >
                {cells[0]?.props.children}
              </div>
              <dl>
                {cells.slice(1).map((cell, index) => (
                  <div
                    className={
                      headers[index + 1] === "操作"
                        ? "data-card-actions"
                        : "data-card-field"
                    }
                    key={index}
                  >
                    <dt>{headers[index + 1]}</dt>
                    <dd className={cell.props.className}>
                      {cell.props.children}
                    </dd>
                  </div>
                ))}
              </dl>
            </article>
          );
        })}
      </div>
    );
  return (
    <div className={`table-frame${empty && !loading ? " is-empty" : ""}`}>
      <Table>
        <TableHeader>
          <TableRow>
            {headers.map((h, index) => (
              <TableHead key={`${index}:${h}`}>{h}</TableHead>
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>
          {loading
            ? [0, 1, 2].map((row) => (
                <TableRow key={row}>
                  {headers.map((header, index) => (
                    <TableCell key={`${index}:${header}`}>
                      <Skeleton className="my-2 h-4 w-3/4" />
                    </TableCell>
                  ))}
                </TableRow>
              ))
            : empty
              ? null
              : children}
        </TableBody>
      </Table>
      {loading && (
        <span className="sr-only" role="status">
          正在加载数据
        </span>
      )}
      {!loading && empty && <ListEmpty />}
    </div>
  );
}
function ListEmpty() {
  return (
    <Empty className="list-empty" role="status" aria-label="暂无数据">
      <EmptyHeader>
        <EmptyMedia>
          <Inbox aria-hidden="true" />
        </EmptyMedia>
        <EmptyTitle>暂无数据</EmptyTitle>
      </EmptyHeader>
    </Empty>
  );
}
export { TableRow as Row, TableCell as Cell };
export const options = (values: string[]) =>
  values.map((value) => ({ value, label: value }));
export function ReadonlyInfo({
  entries,
}: {
  entries: Record<string, unknown>;
}) {
  return (
    <dl className="facts">
      {Object.entries(entries).map(([k, v]) => (
        <div key={k}>
          <dt>{k}</dt>
          <dd>{v === true ? "是" : v === false ? "否" : String(v ?? "—")}</dd>
        </div>
      ))}
    </dl>
  );
}
