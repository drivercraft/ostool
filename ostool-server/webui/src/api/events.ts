import { loginUrl, authSurface } from "@/auth-session";
import { api, refreshAuth, isPublicEdition, ApiRequestError } from "./client";
import { useSyncExternalStore } from "react";
import type {
  AdminOverviewResponse,
  AdminServerConfigResponse,
  BoardConfig,
  AdminBoardSummary,
  AdminSession,
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
  listing: import("@/types/api").ListPage<
    | import("@/types/api").ManagedUser
    | import("@/types/api").Rental
    | import("@/types/api").RentalRequest
  >;
  rental_editor: import("@/types/api").RentalEditorData;
  device_catalog: import("@/types/api").PublicDevice[];
  site: import("@/types/api").SiteView;
  legal_history: import("@/types/api").LegalVersion[];
  mail_deliveries: import("@/types/api").MailDelivery[];
  security: import("./client").SecurityStatus;
  security_sessions: { sessions: import("./client").SecuritySession[] };
  security_revision: number;
  audit: import("@/types/api").AuditPage;
  session_history: import("@/types/api").LeasePage;
  dashboard: import("@/types/api").DashboardInfo;
  profile: import("./client").AuthUser;
  catalog: import("@/types/api").CatalogBoard[];
  availability_query: import("@/types/api").RentalEditorFilters;
  availability: import("@/utils/rental-calendar").RentalWindow[];
  account_summary: { active: number; scheduled: number; pending: number };
  personal_tokens: import("@/types/api").PersonalTokenSummary[];
  rental_requests: { revision: number };
  rentals: { revision: number };
  users: { revision: number };
  roles: import("@/types/api").UserRole[];
  deployment: import("@/types/api").DeploymentInfo;
  account_settings: import("@/types/api").AccountSettingsView;

  boards: AdminBoardSummary[];
  board_editor: { id: string; board: BoardConfig | null };
  sessions: AdminSession[];
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
export type Connection = "connecting" | "connected" | "reconnecting";
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
  private topics: string[] | null = null;
  setTopics = (topics: string[]) => {
    const next = [...new Set(topics)].sort();
    if (JSON.stringify(next) === JSON.stringify(this.topics)) return;
    this.topics = next;
    if (this.source) {
      this.disconnect();
      this.connect();
    }
  };
  private boardEditorId: string | null = null;
  setBoardEditor = (id: string | null) => {
    if (id === this.boardEditorId) return;
    this.boardEditorId = id;
    delete this.data.board_editor;
    delete this.errors.board_editor;
    this.emit();
    if (this.source) {
      this.disconnect();
      this.connect();
    }
  };
  private listingFilters: import("@/types/api").ListFilters | null = null;
  setListingFilters = (filters: import("@/types/api").ListFilters | null) => {
    if (JSON.stringify(filters) === JSON.stringify(this.listingFilters)) return;
    this.listingFilters = filters;
    if (this.source) {
      this.disconnect();
      this.connect();
    }
  };
  private editorFilters: import("@/types/api").RentalEditorFilters | null =
    null;
  setEditorFilters = (
    filters: import("@/types/api").RentalEditorFilters | null,
  ) => {
    if (JSON.stringify(filters) === JSON.stringify(this.editorFilters)) return;
    this.editorFilters = filters;
    if (this.source) {
      this.disconnect();
      this.connect();
    }
  };
  private leaseFilters: import("@/types/api").LeaseFilters | null = null;
  setLeaseFilters = (filters: import("@/types/api").LeaseFilters | null) => {
    if (JSON.stringify(filters) === JSON.stringify(this.leaseFilters)) return;
    this.leaseFilters = filters;
    if (this.source) {
      this.disconnect();
      this.connect();
    }
  };
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
  private retryTimer?: ReturnType<typeof setTimeout>;
  private retryDelay = 1000;
  private generation = 0;
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
    this.retryDelay = 1000;
    this.emit();
  };
  connect = () => {
    if (this.source) return;
    const parameters = new URLSearchParams();
    if (this.topics) parameters.set("topics", this.topics.join(","));
    if (isPublicEdition()) parameters.set("surface", authSurface());
    if (this.auditFilters) {
      parameters.set("audit_view", "1");
      for (const [key, value] of Object.entries(this.auditFilters)) {
        if (value !== undefined && value !== "")
          parameters.set(`audit_${key}`, String(value));
      }
    }
    if (this.leaseFilters) {
      parameters.set("lease_view", "1");
      for (const [key, value] of Object.entries(this.leaseFilters)) {
        if (value !== undefined && value !== "")
          parameters.set(`lease_${key}`, String(value));
      }
    }
    if (this.listingFilters) {
      for (const [key, value] of Object.entries(this.listingFilters))
        if (value !== undefined && value !== "")
          parameters.set(`list_${key}`, String(value));
    }
    if (this.boardEditorId)
      parameters.set("board_editor_id", this.boardEditorId);
    if (this.editorFilters) {
      parameters.set("editor_view", "1");
      for (const [key, value] of Object.entries(this.editorFilters))
        if (value !== undefined && value !== "")
          parameters.set(`editor_${key}`, String(value));
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
      void this.recover(source, true);
    });
    this.source.onerror = () => {
      if (this.source !== source) return;
      this.connection = "reconnecting";
      this.emit();
      // Native EventSource retries transport interruptions, but closes permanently
      // after rejected HTTP responses, including an expired access cookie.
      if (source.readyState === EventSource.CLOSED) {
        if (isPublicEdition() && this.endpoint !== "/api/v1/site/events") {
          void this.recover(source, false);
        } else {
          this.disconnect();
          this.retry();
        }
      }
    };
  };
  private retry() {
    const generation = this.generation;
    this.retryTimer = setTimeout(() => {
      this.retryTimer = undefined;
      if (generation === this.generation) this.connect();
    }, this.retryDelay);
    this.retryDelay = Math.min(this.retryDelay * 2, 30000);
  }
  private async recover(source: EventSource, refresh: boolean) {
    if (this.source !== source) return;
    this.disconnect();
    const generation = this.generation;
    this.connection = "reconnecting";
    this.emit();
    try {
      if (refresh) await refreshAuth();
      const user = await api.me();
      if (generation !== this.generation) return;
      if (!this.member && !user.is_admin) {
        window.location.assign("/admin/login");
        return;
      }
      this.connect();
    } catch (error) {
      if (generation !== this.generation) return;
      if (error instanceof ApiRequestError && error.status === 401) {
        window.location.assign(loginUrl(authSurface()));
      } else {
        // A restart/outage must not erase a valid persistent login or editor draft.
        this.retry();
      }
    }
  }
  disconnect = () => {
    ++this.generation;
    clearTimeout(this.retryTimer);
    this.retryTimer = undefined;
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

export const site = new AdminStore("/api/v1/site/events", true);
export function useSiteResource() {
  return useSyncExternalStore(site.subscribe, () => site.get("site"));
}
export function usePublicDevices() {
  return useSyncExternalStore(site.subscribe, () => site.get("device_catalog"));
}
