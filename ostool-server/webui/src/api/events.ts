import { loginUrl, authSurface } from "@/auth-session";
import { api, refreshAuth, isPublicEdition } from "./client";
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
  security: import("./client").SecurityStatus;
  security_sessions: { sessions: import("./client").SecuritySession[] };
  security_revision: number;
  audit: import("@/types/api").AuditPage;
  dashboard: import("@/types/api").DashboardInfo;
  profile: import("./client").AuthUser;
  catalog: import("@/types/api").CatalogBoard[];
  availability: import("@/utils/rental-calendar").RentalWindow[];
  my_rentals: import("@/types/api").Rental[];
  my_requests: import("@/types/api").RentalRequest[];
  personal_tokens: import("@/types/api").PersonalTokenSummary[];
  rental_requests: import("@/types/api").RentalRequest[];
  rentals: import("@/types/api").Rental[];
  users: import("@/types/api").ManagedUser[];
  roles: import("@/types/api").UserRole[];
  deployment: import("@/types/api").DeploymentInfo;

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
  private auditFilters: import("@/types/api").AuditFilters | null = null;
  setAuditFilters = (filters: import("@/types/api").AuditFilters | null) => {
    if (JSON.stringify(filters) === JSON.stringify(this.auditFilters)) return;
    this.auditFilters = filters;
    if (this.source) {
      this.disconnect();
      this.connect();
    }
  };
  constructor(
    private endpoint = "/api/v1/admin/events",
    private member = false,
  ) {}
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
    const parameters = new URLSearchParams();
    if (isPublicEdition()) parameters.set("surface", authSurface());
    if (this.auditFilters) {
      parameters.set("audit_view", "1");
      for (const [key, value] of Object.entries(this.auditFilters)) {
        if (value !== undefined && value !== "")
          parameters.set(`audit_${key}`, String(value));
      }
    }
    const suffix = parameters.toString() ? `?${parameters}` : "";
    this.source = new EventSource(`${this.endpoint}${suffix}`);
    const source = this.source;
    const receive = (event: MessageEvent) => {
      if (this.source !== source) return;
      try {
        this.apply(JSON.parse(event.data));
      } catch {
        this.restart();
      }
    };
    this.source.addEventListener("snapshot", receive as EventListener);
    this.source.addEventListener("update", receive as EventListener);
    this.source.addEventListener("auth-expired", () => {
      if (this.source !== source) return;
      this.disconnect();
      void refreshAuth()
        .then(() => api.me())
        .then((user) => {
          if (!this.member && !user.is_admin) {
            window.location.assign("/admin/login");
            return;
          }
          this.restart();
        })
        .catch(() => window.location.assign(loginUrl(authSurface())));
    });
    this.source.onopen = () => {
      if (this.source !== source) return;
      if (this.epoch) {
        this.connection = "connected";
        this.emit();
      }
    };
    this.source.onerror = () => {
      if (this.source !== source) return;
      this.connection = "reconnecting";
      this.emit();
    };
  };
  disconnect = () => {
    this.source?.close();
    this.source = undefined;
  };
  reset = () => {
    this.disconnect();
    this.data = {};
    this.errors = {};
    this.epoch = "";
    this.revision = 0;
    this.connection = "connecting";
    this.emit();
  };
  restart = () => {
    this.disconnect();
    this.connection = "reconnecting";
    this.emit();
    this.connect();
  };
}
export const admin = new AdminStore();
export const account = new AdminStore("/api/v1/account/events", true);
export function useAccountResource<K extends keyof Resources>(topic: K) {
  return useSyncExternalStore(account.subscribe, () => account.get(topic));
}
export function useAccountConnection() {
  return useSyncExternalStore(account.subscribe, account.getConnection);
}
export function useAccountErrors() {
  return useSyncExternalStore(account.subscribe, account.getErrors);
}
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

export const security = new AdminStore("/api/v1/auth/security/events", true);
export function useSecurityResource<K extends "security" | "security_sessions">(
  topic: K,
) {
  return useSyncExternalStore(security.subscribe, () => security.get(topic));
}
export function useSecurityErrors() {
  return useSyncExternalStore(security.subscribe, security.getErrors);
}
