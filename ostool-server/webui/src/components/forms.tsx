import {
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
import { Inbox } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
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

export function TextField({
  label,
  hint,
  onValue,
  ...props
}: ComponentProps<typeof Input> & {
  label: string;
  hint?: string;
  onValue?: (value: string) => void;
}) {
  const id = useId();
  return (
    <Field>
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
      <Input
        id={id}
        {...props}
        onChange={onValue ? (e) => onValue(e.target.value) : props.onChange}
      />
      {hint && <FieldDescription>{hint}</FieldDescription>}
    </Field>
  );
}
export function TextareaField({
  label,
  value,
  onValue,
  hint,
}: {
  label: string;
  value: string;
  onValue: (value: string) => void;
  hint?: string;
}) {
  const id = useId();
  return (
    <Field>
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
      <Textarea
        id={id}
        value={value}
        onChange={(e) => onValue(e.target.value)}
        rows={2}
      />
      {hint && <FieldDescription>{hint}</FieldDescription>}
    </Field>
  );
}
export function SelectField({
  label,
  options,
  onValue,
  ...props
}: Omit<ComponentProps<typeof NativeSelect>, "onChange"> & {
  label: string;
  options: { value: string; label: string; disabled?: boolean }[];
  onValue: (value: string) => void;
}) {
  const id = useId();
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
        onChange={(e) => onValue(e.target.value)}
      >
        {optionsWithCurrent.map((o) => (
          <option key={o.value} value={o.value} disabled={o.disabled}>
            {o.label}
          </option>
        ))}
      </NativeSelect>
    </Field>
  );
}
export function CheckField({
  label,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  checked: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
}) {
  const id = useId();
  return (
    <Field orientation="horizontal">
      <Checkbox
        id={id}
        checked={checked}
        onCheckedChange={(value) => onChange(value === true)}
        disabled={disabled}
      />
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
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
}: {
  label: string;
  description: string;
  action: () => Promise<unknown>;
  disabled?: boolean;
  triggerVariant?: "ghost" | "destructive";
}) {
  const [open, setOpen] = useState(false);
  const task = useAction();
  return (
    <>
      <Button
        type="button"
        variant={triggerVariant}
        size="sm"
        disabled={disabled}
        onClick={() => setOpen(true)}
      >
        {label}
      </Button>
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
export function DataTable({
  headers,
  children,
  empty,
}: {
  headers: string[];
  children: ReactNode;
  empty?: boolean;
}) {
  const view = useListLayout();
  if (!view)
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
  if (view.layout === "cards" && empty) return <ListEmpty />;
  if (view.layout === "cards")
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
    <div className={`table-frame${empty ? " is-empty" : ""}`}>
      <Table>
        <TableHeader>
          <TableRow>
            {headers.map((h, index) => (
              <TableHead key={`${index}:${h}`}>{h}</TableHead>
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>{empty ? null : children}</TableBody>
      </Table>
      {empty && <ListEmpty />}
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
