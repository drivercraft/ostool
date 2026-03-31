export interface ErrorResponse {
  code: string;
  message: string;
  details?: unknown;
}

export interface LeaseConfig {
  default_ttl_secs: number;
  max_ttl_secs: number;
  gc_interval_secs: number;
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

export interface TftpStatus {
  provider: string;
  enabled: boolean;
  healthy: boolean;
  writable: boolean;
  root_dir: string;
  bind_addr_or_address: string | null;
  service_state: string | null;
  last_error: string | null;
}

export interface SerialConfig {
  port: string;
  baud_rate: number;
}

export interface UbootNetConfig {
  interface: string;
  board_ip: string | null;
  gatewayip: string | null;
  netmask: string | null;
  server_ip_override: string | null;
}

export interface UbootProfile {
  kind: "uboot";
  kernel_load_addr: string | null;
  fit_load_addr: string | null;
  net: UbootNetConfig | null;
  board_reset_cmd: string | null;
  board_power_off_cmd: string | null;
  success_regex: string[];
  fail_regex: string[];
  uboot_cmd: string[] | null;
  shell_prefix: string | null;
  shell_init_cmd: string | null;
  timeout: number | null;
}

export interface PxeProfile {
  kind: "pxe";
  notes: string | null;
}

export type BootConfig = UbootProfile | PxeProfile;

export interface BoardConfig {
  id: string;
  name: string;
  board_type: string;
  tags: string[];
  serial: SerialConfig | null;
  boot: BootConfig;
  notes: string | null;
  disabled: boolean;
}

export interface BoardTypeSummary {
  board_type: string;
  tags: string[];
  total: number;
  available: number;
}

export interface Session {
  id: string;
  board_id: string;
  client_name: string | null;
  created_at: string;
  expires_at: string;
}

export interface AdminSessionsResponse {
  sessions: Session[];
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
  listen_addr: string;
  data_dir: string;
  board_dir: string;
}

export interface AdminServerConfigEditable {
  lease: LeaseConfig;
}

export interface AdminServerConfigResponse {
  readonly: AdminServerConfigReadonly;
  editable: AdminServerConfigEditable;
}

export interface UpdateServerConfigRequest {
  lease: LeaseConfig;
}

export interface FileResponse {
  slot: string;
  filename: string;
  relative_path: string;
  tftp_url: string | null;
  size: number;
  uploaded_at: string;
}

export interface TftpSessionResponse {
  available: boolean;
  provider: string;
  server_ip: string | null;
  writable: boolean;
  files: FileResponse[];
}
