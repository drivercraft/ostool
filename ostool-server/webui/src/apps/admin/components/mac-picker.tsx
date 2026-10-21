import { useId, useState } from "react";
import { ChevronDownIcon } from "lucide-react";
import type { LoaderDeviceSummary } from "@/types/api";
import { Field, FieldLabel, FieldDescription } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
export function MacPicker({
  value,
  onChange,
  devices,
  boardId,
}: {
  value: string;
  onChange: (value: string) => void;
  devices: LoaderDeviceSummary[];
  boardId?: string;
}) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const candidates = devices.filter(
    (d) => !d.bound_board_id || d.bound_board_id === boardId,
  );
  return (
    <Field>
      <FieldLabel htmlFor={id}>MAC 地址</FieldLabel>
      <div className="flex gap-2">
        <Input
          id={id}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder="选择或手动输入 MAC 地址"
        />
        <Popover open={open} onOpenChange={setOpen}>
          <PopoverTrigger asChild>
            <Button
              type="button"
              variant="outline"
              aria-label={`发现 ${candidates.length} 台设备`}
            >
              发现 {candidates.length} 台
              <ChevronDownIcon data-icon="inline-end" />
            </Button>
          </PopoverTrigger>
          <PopoverContent align="end" className="w-80 max-h-80 overflow-y-auto">
            <p className="hint">已发现设备</p>
            {candidates.length === 0 ? (
              <p>尚未发现设备，可先手动上电。</p>
            ) : (
              candidates.map((d) => (
                <Button
                  key={d.mac_address}
                  type="button"
                  variant="ghost"
                  className="h-auto w-full justify-start py-3"
                  disabled={d.conflict}
                  onClick={() => {
                    onChange(d.mac_address);
                    setOpen(false);
                  }}
                >
                  <span className="text-left">
                    <span className="mono">{d.mac_address}</span>
                    <small>
                      {d.ip_address} · {d.arch} ·{" "}
                      {d.conflict ? "MAC 冲突" : d.online ? "在线" : "离线"}
                    </small>
                    <small>
                      {d.hardware.manufacturer} {d.hardware.product}
                    </small>
                    <small>
                      {d.bound_board_id
                        ? `已绑定 ${d.bound_board_id}`
                        : "未绑定"}
                    </small>
                  </span>
                </Button>
              ))
            )}
          </PopoverContent>
        </Popover>
      </div>
      <FieldDescription>
        发现列表实时更新；只有保存配置后才完成绑定。
      </FieldDescription>
    </Field>
  );
}
