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

export type SerialPortKeyKind = "serial_number" | "usb_path";

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
  serial_port: string;
}

export type PowerManagementConfig = CustomPowerManagement | ZhongshengRelayPowerManagement;

export interface UbootProfile {
  kind: "uboot";
  use_tftp: boolean;
  dtb_name: string | null;
}

export interface PxeProfile {
  kind: "pxe";
  notes: string | null;
}

export type BootConfig = UbootProfile | PxeProfile;

export interface BoardConfig {
  id: string;
  board_type: string;
  tags: string[];
  serial: SerialConfig | null;
  power_management: PowerManagementConfig;
  boot: BootConfig;
  notes: string | null;
  disabled: boolean;
}

export interface AdminBoardUpsertRequest {
  id: string | null;
  board_type: string;
  tags: string[];
  notes: string | null;
  disabled: boolean;
  serial: SerialConfig | null;
  power_management: PowerManagementConfig;
  boot: BootConfig;
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
  dtb_dir: string;
}

export interface AdminServerConfigEditable {
  network: TftpNetworkConfig;
}

export interface AdminServerConfigResponse {
  readonly: AdminServerConfigReadonly;
  editable: AdminServerConfigEditable;
}

export interface UpdateServerConfigRequest {
  network: TftpNetworkConfig;
}

export interface BootProfileResponse {
  boot: BootConfig;
  server_ip: string | null;
  netmask: string | null;
  interface: string | null;
}

export interface FileResponse {
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
  netmask: string | null;
  writable: boolean;
  files: FileResponse[];
}
