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
}

export type BootConfig = UbootProfile | PxeProfile | UefiHttpProfile;

export interface BoardConfig {
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
  phase: "queued" | "downloading" | "staged" | "confirming" | "succeeded" | "rolled_back" | "failed" | "cancelled";
  error: string | null;
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
  id: string;
  board_id: string;
  client_name: string | null;
  created_at: string;
  expires_at: string;
  state: "active" | "releasing";
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
  http_boot_public_base_url: string | null;
  listen_addr: string;
  data_dir: string;
  board_dir: string;
  dtb_dir: string;
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
