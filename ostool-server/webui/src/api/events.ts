import { useSyncExternalStore } from "react";
import type {
  AdminOverviewResponse,
  AdminServerConfigResponse,
  BoardConfig,
  Session,
  LoaderDeviceSummary,
  LoaderOtaResources,
  VirtualDevicesResponse,
  DtbFileResponse,
  SerialPortSummary,
  NetworkInterfaceSummary,
  TftpConfig,
  TftpStatus,
} from "@/types/api";
export interface PowerResult {
  id: string;
  action: "on" | "off";
  state: "running" | "succeeded" | "failed";
  message: string | null;
}
export interface Runtime {
  lease_state: "idle" | "using" | "releasing" | "error";
  active_session_id: string | null;
  last_release_error: string | null;
}
export interface Resources {
  quarantined_boards: {
    original_path: string;
    backup_path: string;
    reason: string;
    quarantined_at: string;
  }[];
  boards: BoardConfig[];
  sessions: Session[];
  runtimes: Record<string, Runtime>;
  loaders: LoaderDeviceSummary[];
  ota: LoaderOtaResources;
  virtual: VirtualDevicesResponse;
  dtbs: DtbFileResponse[];
  serial: SerialPortSummary[];
  serial_manager: { pending: number; candidates: number; leased: number };
  network: NetworkInterfaceSummary[];
  server: AdminServerConfigResponse;
  tftp: TftpConfig;
  tftp_status: TftpStatus;
  overview: AdminOverviewResponse;
  power_actions: PowerResult[];
}
export interface Envelope {
  epoch: string;
  revision: number;
  kind: "snapshot" | "update";
  data: Partial<Resources>;
}
type Connection = "connecting" | "connected" | "reconnecting";
const empty: unknown[] = [];
// Structural reconciliation retains unchanged rows across snapshots and reconnects.
function reconcile(previous: unknown, next: unknown): unknown {
  if (Object.is(previous, next)) return previous;
  if (Array.isArray(next)) {
    const old = Array.isArray(previous) ? previous : [];
    const key = (v: unknown, index: number) =>
      typeof v === "object" && v !== null
        ? ((v as Record<string, unknown>).id ??
          (v as Record<string, unknown>).mac_address ??
          (v as Record<string, unknown>).name ??
          index)
        : index;
    const byId = new Map(old.map((v, i) => [key(v, i), v]));
    const result = next.map((v, i) => reconcile(byId.get(key(v, i)), v));
    return old.length === result.length && result.every((v, i) => v === old[i])
      ? old
      : result;
  }
  if (next && typeof next === "object") {
    const old =
      previous && typeof previous === "object"
        ? (previous as Record<string, unknown>)
        : {};
    const result = Object.fromEntries(
      Object.entries(next).map(([k, v]) => [k, reconcile(old[k], v)]),
    );
    return Object.keys(old).length === Object.keys(result).length &&
      Object.keys(result).every((k) => result[k] === old[k])
      ? old
      : result;
  }
  return next;
}
export class AdminStore {
  private data: Partial<Resources> = {};
  private listeners = new Set<() => void>();
  private source?: EventSource;
  private epoch = "";
  private revision = 0;
  private connection: Connection = "connecting";
  private errors: Record<string, string> = {};
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  get = <K extends keyof Resources>(topic: K): Resources[K] | undefined =>
    this.data[topic];
  getConnection = () => this.connection;
  getErrors = () => this.errors;
  private emit() {
    this.listeners.forEach((fn) => fn());
  }
  apply = (event: Envelope) => {
    if (
      event.kind === "update" &&
      (event.epoch !== this.epoch || event.revision !== this.revision + 1)
    ) {
      if (event.epoch === this.epoch && event.revision <= this.revision) return;
      this.restart();
      return;
    }
    this.epoch = event.epoch;
    this.revision = event.revision;
    const data = { ...this.data } as Record<string, unknown>;
    const errors = { ...this.errors };
    for (const [topic, value] of Object.entries(event.data)) {
      if (value && typeof value === "object" && "error" in value) {
        errors[topic] = String(value.error);
        continue;
      }
      delete errors[topic];
      data[topic] = reconcile(data[topic], value);
    }
    this.data = data as Partial<Resources>;
    this.errors = reconcile(this.errors, errors) as Record<string, string>;
    this.connection = "connected";
    this.emit();
  };
  connect = () => {
    if (this.source) return;
    this.source = new EventSource("/api/v1/admin/events");
    const receive = (event: MessageEvent) => {
      try {
        this.apply(JSON.parse(event.data));
      } catch {
        this.restart();
      }
    };
    this.source.addEventListener("snapshot", receive as EventListener);
    this.source.addEventListener("update", receive as EventListener);
    this.source.onopen = () => {
      if (this.epoch) {
        this.connection = "connected";
        this.emit();
      }
    };
    this.source.onerror = () => {
      this.connection = "reconnecting";
      this.emit();
    };
  };
  disconnect = () => {
    this.source?.close();
    this.source = undefined;
  };
  restart = () => {
    this.disconnect();
    this.connection = "reconnecting";
    this.emit();
    this.connect();
  };
}
export const admin = new AdminStore();
export function useResource<K extends keyof Resources>(topic: K) {
  return useSyncExternalStore(admin.subscribe, () => admin.get(topic));
}
export function useList<K extends keyof Resources>(topic: K): Resources[K] {
  return (useResource(topic) ?? empty) as Resources[K];
}
export function useConnection() {
  return useSyncExternalStore(admin.subscribe, admin.getConnection);
}
export function useErrors() {
  return useSyncExternalStore(admin.subscribe, admin.getErrors);
}
