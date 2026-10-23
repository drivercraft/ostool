export interface ErrorResponse {
  code: string;
  message: string;
  details?: unknown;
}

export interface BuiltinTftpConfig {
  provider: "builtin";
  enabled: boolean;
  root_dir: string;
  bind_addr: string;
}

export interface SystemTftpdHpaConfig {
  provider: "system_tftpd_hpa";
  enabled: boolean;
  root_dir: string;
  config_path: string;
  service_name: string;
  username: string | null;
  address: string;
  options: string;
  manage_config: boolean;
  reconcile_on_start: boolean;
}

export type TftpConfig = BuiltinTftpConfig | SystemTftpdHpaConfig;

export interface TftpNetworkConfig {
  interface: string;
}

export interface UploadLimitsConfig {
  session_file_max_mib: number;
  session_total_max_mib: number;
  session_max_files: number;
  storage_max_mib: number;
  max_concurrent_uploads: number;
  user_max_concurrent_uploads: number;
  user_total_max_mib: number;
  upload_timeout_secs: number;
}

export interface TftpStatus {
  provider: string;
  enabled: boolean;
  healthy: boolean;
  writable: boolean;
  resolved_server_ip: string | null;
  resolved_netmask: string | null;
  root_dir: string;
  bind_addr_or_address: string | null;
  service_state: string | null;
  last_error: string | null;
}

export type SerialPortKeyKind = "serial_number" | "usb_path" | "qemu";

export interface SerialPortKey {
  kind: SerialPortKeyKind;
  value: string;
}

export interface SerialConfig {
  key: SerialPortKey;
  baud_rate: number;
  resolved_device_path?: string | null;
  resolved_usb_path?: string | null;
}

export interface SerialPortSummary {
  current_device_path: string;
  port_type: string;
  label: string;
  primary_key_kind: SerialPortKeyKind | null;
  primary_key_value: string | null;
  usb_path: string | null;
  stable_identity: boolean;
  usb_vendor_id: number | null;
  usb_product_id: number | null;
  manufacturer: string | null;
  product: string | null;
  serial_number: string | null;
}

export interface NetworkInterfaceSummary {
  name: string;
  label: string;
  ipv4_addresses: string[];
  netmask: string | null;
  loopback: boolean;
}

export interface CustomPowerManagement {
  kind: "custom";
  power_on_cmd: string;
  power_off_cmd: string;
}

export interface ZhongshengRelayPowerManagement {
  kind: "zhongsheng_relay";
  key: SerialPortKey;
}

export interface QemuPowerManagement {
  kind: "qemu";
  virtual_device_id: string;
}

export type PowerManagementConfig =
  CustomPowerManagement | ZhongshengRelayPowerManagement | QemuPowerManagement;

export type UbootNetworkMode = "dhcp" | "static_ip";

export interface UbootProfile {
  kind: "uboot";
  use_tftp: boolean;
  dtb_name: string | null;
  kernel_load_addr: string | null;
  fit_load_addr: string | null;
  bootm_addr: string | null;
  network_mode: UbootNetworkMode;
  board_ip: string | null;
  server_ip: string | null;
  netmask: string | null;
  gatewayip: string | null;
}

export interface PxeProfile {
  kind: "pxe";
  notes: string | null;
}

export interface UefiHttpProfile {
  kind: "httpboot";
  boot_arch?: string | null;
  serial_parameters?: AxloaderSerialParameters | null;
}

export type AxloaderSerialParity = "none" | "odd" | "even" | "mark" | "space";
export type AxloaderSerialStopBits = "one" | "one_point_five" | "two";
export type AxloaderSerialFlowControl = "none" | "rts_cts";

export interface AxloaderSerialParameters {
  baud_rate: number;
  data_bits: number;
  parity: AxloaderSerialParity;
  stop_bits: AxloaderSerialStopBits;
  flow_control: AxloaderSerialFlowControl;
}

export type BootConfig = UbootProfile | PxeProfile | UefiHttpProfile;

export interface BoardConfig {
  reset_command?: string | null;
  id: string;
  board_type: string;
  tags: string[];
  serial: SerialConfig | null;
  power_management: PowerManagementConfig;
  boot: BootConfig;
  network_identity: BoardNetworkIdentity | null;
  notes: string | null;
  disabled: boolean;
}

export interface AdminBoardUpsertRequest {
  reset_command?: string | null;
  id: string | null;
  board_type: string;
  tags: string[];
  notes: string | null;
  disabled: boolean;
  serial: SerialConfig | null;
  power_management: PowerManagementConfig;
  boot: BootConfig;
  network_identity: BoardNetworkIdentity | null;
}

export interface BoardNetworkIdentity {
  mac_address: string;
}

export interface LoaderHardwareInfo {
  manufacturer: string | null;
  product: string | null;
  version: string | null;
  serial: string | null;
}

export interface LoaderDeviceSummary {
  mac_address: string;
  current_mac_address: string;
  ip_address: string;
  arch: string;
  loader_version: string;
  hardware: LoaderHardwareInfo;
  last_seen_at: string;
  online: boolean;
  conflict: boolean;
  bound_board_id: string | null;
  current_registration_id: string | null;
}

export interface LoaderImage {
  sha256: string;
  size: number;
  version: string | null;
}

export interface LoaderUpdateJob {
  board_id: string;
  mac_address: string;
  update_id: string;
  image: LoaderImage;
  phase:
    | "queued"
    | "downloading"
    | "staged"
    | "confirming"
    | "succeeded"
    | "rolled_back"
    | "failed"
    | "cancelled";
  error: string | null;
  delivery_attempts: number;
}

export interface LoaderOtaResources {
  jobs: LoaderUpdateJob[];
  images: LoaderImage[];
}

export interface VirtualDeviceSummary {
  id: string;
  mac_address: string;
  tap: string;
  powered: boolean;
  serial_connected: boolean;
  generation: number;
}

export interface VirtualDevicesResponse {
  enabled: boolean;
  devices: VirtualDeviceSummary[];
}

export interface BoardTypeSummary {
  board_type: string;
  tags: string[];
  total: number;
  available: number;
}

export interface DtbFileResponse {
  name: string;
  size: number;
  updated_at: string;
  relative_tftp_path_template: string;
}

export interface Session {
  serial_connected?: boolean;
  serial_runtime?: SerialRuntimeStatus;
  id: string;
  board_id: string;
  client_name: string | null;
  created_at: string;
  expires_at: string;
  state: "active" | "releasing";
}

export interface AdminSession extends Session {
  owner_username: string | null;
  owner_user_id: string | null;
  rental_id: string | null;
  rental_starts_at: string | null;
  rental_ends_at: string | null;
}
export interface AdminSessionsResponse {
  sessions: AdminSession[];
}

export interface AdminTftpConfigResponse {
  tftp: TftpConfig;
}

export interface AdminTftpStatusResponse {
  status: TftpStatus;
}

export interface AdminOverviewResponse {
  board_count_total: number;
  board_count_available: number;
  disabled_board_count: number;
  active_session_count: number;
  board_types: BoardTypeSummary[];
  tftp_status: TftpStatus;
  server: AdminServerConfigReadonly;
}

export interface AdminServerConfigReadonly {
  http_boot_public_base_url: string | null;
  listen_addr: string;
  data_dir: string;

  dtb_upload_max_mib: number;
}

export interface AdminServerConfigEditable {
  network: TftpNetworkConfig;
  upload_limits: UploadLimitsConfig;
}

export interface AdminServerConfigResponse {
  readonly: AdminServerConfigReadonly;
  editable: AdminServerConfigEditable;
}

export interface UpdateServerConfigRequest {
  network: TftpNetworkConfig;
  upload_limits: UploadLimitsConfig;
}

export interface BootProfileResponse {
  server_side_operations: boolean;
  http_base_url: string | null;
  boot: BootConfig;
  server_ip: string | null;
  netmask: string | null;
  interface: string | null;
}

export interface UbootPrepareRequest {
  commands?: string[];
  kernel_load_addr?: number | null;
  fit_load_addr?: number | null;
  bootm_addr?: number | null;
  timeout_secs?: number | null;
}
export interface UbootPrepared {
  kernel_load_addr: number;
  fit_load_addr: number;
  fdt_load_addr: number | null;
}
export interface UbootBootRequest {
  path: string;
  cmdline?: string | null;
}

export interface FileResponse {
  filename: string;
  relative_path: string;
  tftp_url: string | null;
  http_url: string | null;
  size: number;
  uploaded_at: string;
}

export interface TftpSessionResponse {
  available: boolean;
  provider: string;
  server_ip: string | null;
  netmask: string | null;
  writable: boolean;
  files: FileResponse[];
}

export interface SerialRuntimeStatus {
  phase:
    | "waiting_device"
    | "verifying"
    | "discovering"
    | "bound"
    | "recovering"
    | "failed"
    | "closed";
  port: string | null;
  parameters: {
    baud_rate: number;
    data_bits: number;
    parity: string;
    stop_bits: string;
    flow_control: string;
  } | null;
  error: string | null;
  warning: string | null;
  binding_id: string | null;
  boot_epoch: string | null;
}

export interface ManagedUser {
  id: string;
  username: string;
  email: string;
  display_name: string | null;
  role_id: string;
  disabled: boolean;
  created_at: string;
  updated_at?: string;
  email_verified_at?: string | null;
  last_login_at?: string | null;
  last_login_ip?: string | null;
  password_changed_at?: string;
  mfa_enabled?: boolean;
}
export interface UserInput {
  username: string;
  email: string;
  display_name: string | null;
  role_id: string;
  disabled: boolean;
  password?: string;
}
export interface RoleInput {
  name: string;
  description: string;
  administrator: boolean;
}
export interface UserRole extends RoleInput {
  user_count: number;
  created_at?: string;
  updated_at?: string;
  id: string;
  builtin: boolean;
}
export interface DeploymentInfo {
  edition: "local" | "public";
  database_connected: boolean;
  redis_connected?: boolean;
  security?: Record<string, string | number | boolean | null>;
  auth_enabled: boolean;
  allow_registration: boolean;
  public_base_url: string | null;
  access_token_ttl_secs: number;
  storage: Record<string, string>;
}

export interface RentalInput {
  board_id: string;
  user_id: string;
  starts_at: string;
  ends_at: string;
  notes: string;
}
export interface Rental extends RentalInput {
  user_username?: string;
  user_email?: string;
  user_display_name?: string | null;
  id: string;
  status: "scheduled" | "active" | "expired" | "cancelled";
  cancelled_at: string | null;
  created_at: string;
}
export interface ListFilters {
  view: "users" | "rentals" | "rental_requests";
  q?: string;
  role?: string;
  status?: string;
  board?: string;
  user?: string;
  page?: number;
  page_size?: number;
}
export interface ListPage<T> {
  active: boolean;
  query: ListFilters;
  items: T[];
  total: number;
  page: number;
  page_size: number;
}
export interface RentalEditorFilters {
  id?: string;
  user?: string;
  q?: string;
  board?: string;
  from?: string;
  to?: string;
}
export interface RentalEditorData {
  query: RentalEditorFilters;
  users: ManagedUser[];
  more_users: boolean;
  rental: Rental | null;
  bookings: import("@/utils/rental-calendar").RentalWindow[];
}
export interface RentalRequest {
  id: string;
  user_id: string;
  user_email: string;
  board_id: string;
  starts_at: string;
  ends_at: string;
  notes: string;
  status: "pending" | "approved" | "rejected" | "cancelled";
  decision_note: string;
  reviewed_at: string | null;
  rental_id: string | null;
  created_at: string;
}
export interface RentalApplication {
  board_id: string;
  starts_at: string;
  ends_at: string;
  notes: string;
}
export interface CatalogBoard {
  id: string;
  board_type: string;
  tags: string[];
}
export interface PublicDevice extends CatalogBoard {
  disabled: boolean;
}
export interface PersonalTokenSummary {
  id: string;
  name: string;
  expires_at: string | null;
  created_at: string;
}
export interface AuditFilters {
  q?: string;
  action?: string;
  resource?: string;
  result?: string;
  source?: string;
  actor?: string;
  ip?: string;
  from?: string;
  to?: string;
  page?: number;
  page_size?: number;
}
export interface AuditEvent {
  id: string;
  request_id: string;
  created_at: string;
  finished_at: string | null;
  actor_id: string | null;
  actor_name: string;
  actor_kind: string;
  source: string;
  action: string;
  summary: string;
  resource: string;
  target_id: string | null;
  method: string | null;
  route: string | null;
  ip: string | null;
  user_agent: string | null;
  result: string;
  status_code: number | null;
  duration_ms: number | null;
  details: Record<string, unknown>;
}
export interface AuditPage {
  query?: AuditFilters;
  active: boolean;
  items?: AuditEvent[];
  total?: number;
  page?: number;
  page_size?: number;
  actions?: { value: string; label: string }[];
  resources?: string[];
  retention_days?: number | null;
}
export interface DashboardTask {
  id: string;
  name: string;
  state: string;
  interval_secs: number;
  runs: number;
  last_run: string | null;
  next_run: string | null;
  last_duration_ms: number | null;
  last_ok: boolean | null;
}
export interface DashboardInfo {
  runtime: {
    started_at: string;
    uptime_secs: number;
    version: string;
    platform: string;
    arch: string;
    pid: number;
    logical_cpus: number | null;
    sample: {
      ready: boolean;
      sampled_at?: string;
      hostname?: string | null;
      cpu_percent?: number | null;
      load_average?: number[] | null;
      memory_total?: number | null;
      memory_available?: number | null;
      process_rss?: number | null;
      disk_total?: number | null;
      disk_available?: number | null;
      interface?: string;
      rx_bytes?: number | null;
      tx_bytes?: number | null;
      rx_bytes_per_second?: number | null;
      tx_bytes_per_second?: number | null;
      redis_ok?: boolean | null;
      database_ok?: boolean;
      database_latency_ms?: number;
      pool_size?: number;
      pool_idle?: number;
    };
    history: {
      at: string;
      cpu_percent: number | null;
      memory_percent: number | null;
      rx: number | null;
      tx: number | null;
    }[];
    tasks: DashboardTask[];
    http: {
      requests: number;
      errors: number;
      inflight: number;
      mean_latency_ms: number;
      sse_connections: number;
    };
  };
  operations: {
    boards: number;
    board_states: Record<string, number>;
    sessions: number;
    devices: number;
    online_devices: number;
    conflicting_devices: number;
    dtbs: number | null;
    public: boolean;
    database_available?: boolean;
    users?: number;
    enabled_users?: number;
    active_rentals?: number;
    scheduled_rentals?: number;
    cancelled_rentals?: number;
    pending_requests?: number;
    recent_audits?: number;
    recent_failures?: number;
  };
  audit: { healthy: boolean; write_failures: number };
  power_actions: {
    id: string;
    action: string;
    state: string;
    message: string | null;
  }[];
  ota_jobs: LoaderUpdateJob[];
  audit_retention_days: number | null;
}

/** Session API never returns administrator-only hardware configuration. */
export interface SessionDetailResponse {
  session: Session;
  board: PublicDevice;
  serial_available: boolean;
  serial_connected: boolean;
  files: FileResponse[];
}

export interface SiteSettings {
  name: string;
  short_name: string;
  tagline: string;
  description: string;
  logo_url: string;
  favicon_url: string;
  support_email: string;
  support_phone: string;
  support_work_hours: string;
  privacy_contact_email: string;
  support_url: string;
  operator: string;
  address: string;
  copyright: string;
  icp_number: string;
  icp_url: string;
  police_number: string;
  police_url: string;
  github_url: string;
  maintenance: boolean;
  maintenance_message: string;
  terms_title: string;
  terms_body: string;
  privacy_title: string;
  privacy_body: string;
}
export interface SiteView {
  revision: number;
  terms_version: number;
  privacy_version: number;
  updated_at: string;
  terms_published_at?: string;
  privacy_published_at?: string;
  settings: SiteSettings;
  auth?: {
    allow_registration: boolean;
    registration_email_verification: boolean;
    email_security_available: boolean;
    password_reset_available: boolean;
  };
}
export interface LegalConsent {
  accepted: boolean;
  terms_version: number;
  privacy_version: number;
}
export interface LegalVersion {
  kind: "terms" | "privacy";
  version: number;
  title: string;
  content_hash: string;
  published_at: string;
}
export interface MailDelivery {
  id: string;
  purpose: string;
  recipient_email: string;
  status: "sending" | "sent" | "failed";
  error_code: string | null;
  created_at: string;
  completed_at: string | null;
}

export const requestStatusLabels: Record<RentalRequest["status"], string> = {
  pending: "待审核",
  approved: "已通过",
  rejected: "已拒绝",
  cancelled: "已撤回",
};

export interface EmailPolicy {
  ttl_secs: number;
  resend_secs: number;
  window_secs: number;
  email_limit: number;
  ip_limit: number;
  max_attempts: number;
}
export interface AccountSettings {
  allow_registration: boolean;
  access_token_ttl_secs: number;
  refresh_token_ttl_secs: number;
  device_code_ttl_secs: number;
  max_active_sessions: number;
  captcha_provider: "image" | "turnstile";
  turnstile_site_key: string | null;
  turnstile_hostnames: string[];
  require_admin_mfa: boolean;
  registration_email_verification: boolean;
  login_max_failures: number;
  login_lock_secs: number;
  smtp_enabled: boolean;
  smtp_host: string | null;
  smtp_port: number;
  smtp_username: string | null;
  smtp_from: string | null;
  smtp_tls: "starttls" | "tls" | "loopback_plaintext";
  password_reset_enabled: boolean;
  password_reset_ttl_secs: number;
  email_policy: EmailPolicy;
  registration_email_policy: EmailPolicy;
}
export interface AccountSettingsView {
  revision: number;
  settings: AccountSettings;
  secrets: {
    smtp_password: boolean;
    turnstile_secret: boolean;
    mfa_key: boolean;
  };
}
export interface LegalDocument extends LegalVersion {
  body: string;
}

export interface LeaseRecord {
  session_id: string;
  board_id: string;
  board_type: string;
  user_id: string | null;
  username: string | null;
  rental_id: string | null;
  client_name: string | null;
  created_at: string;
  last_heartbeat_at: string;
  expires_at: string;
  stop_requested_at: string | null;
  ended_at: string | null;
  stop_reason: string | null;
  end_reason: string | null;
  release_failures: number;
  last_release_error: string | null;
}
export interface LeaseFilters {
  q?: string;
  board?: string;
  state?: string;
  reason?: string;
  from?: string;
  to?: string;
  page?: number;
  page_size?: number;
}
export interface LeasePage {
  active: boolean;
  query?: LeaseFilters;
  items?: LeaseRecord[];
  total?: number;
  page?: number;
  page_size?: number;
}

export interface SecretUpdates {
  smtp_password?: string;
  turnstile_secret?: string;
  clear_smtp_password?: boolean;
  clear_turnstile_secret?: boolean;
}
