export interface ErrorResponse {
  code: string;
  message: string;
  details?: unknown;
}

export interface CurrentUserResponse {
  id: string;
  username: string;
  display_name: string;
  nickname: string | null;
  avatar_url: string | null;
  email: string;
  phone: string | null;
  department: string | null;
  title: string | null;
  last_login_at: string | null;
  roles: AdminRoleResponse[];
  permissions: AdminPermissionResponse[];
}

export interface LoginRequest {
  username: string;
  password: string;
  captcha_token: string;
  captcha_answer: string;
}

export interface RegisterRequest {
  username: string;
  display_name?: string;
  email: string;
  password: string;
  confirm_password: string;
  captcha_token: string;
  captcha_answer: string;
  phone?: string;
  department?: string;
  title?: string;
}

export type RegisterOutcome = "closed" | "active" | "pending";

export type RegisterResponse =
  | { outcome: "closed" }
  | { outcome: "active"; username: string; display_name: string }
  | { outcome: "pending"; username: string; display_name: string };

export interface RegistrationPolicyResponse {
  mode: "closed" | "auto" | "approval";
  self_service_enabled: boolean;
}

export interface CaptchaResponse {
  token: string;
  image_svg: string;
  expires_in_seconds: number;
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
  key: SerialPortKey;
}

export type PowerManagementConfig =
  | CustomPowerManagement
  | ZhongshengRelayPowerManagement;

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
  storage_path?: string | null;
  sha256?: string | null;
  boot_architecture?: string | null;
  compatible?: string | null;
  description?: string | null;
  disabled: boolean;
  relative_tftp_path_template: string;
}

export interface DtbMetadataInput {
  boot_architecture?: string | null;
  compatible?: string | null;
  description?: string | null;
  disabled?: boolean | null;
}

export interface Session {
  id: string;
  board_id: string;
  client_name: string | null;
  source_ip: string | null;
  created_at: string;
  expires_at: string;
  state: "active" | "releasing";
}

export type SessionRecordState = "active" | "releasing" | "released" | "expired" | "failed";

export interface SessionRecord {
  id: string;
  board_id: string;
  client_name: string | null;
  source_ip: string | null;
  state: SessionRecordState;
  created_at: string;
  last_heartbeat_at: string;
  expires_at: string;
  ended_at: string | null;
  failure_message: string | null;
}

export interface AdminSessionResponse {
  session: SessionRecord;
  lease: Lease | null;
  user_id: string | null;
  source_ip: string | null;
}

export interface AdminSessionUpdateRequest {
  client_name: string | null;
  failure_message: string | null;
}

export type LeaseState = "active" | "releasing" | "released" | "expired" | "failed";

export interface Lease {
  id: string;
  user_id: string;
  session_id: string | null;
  board_id: string;
  board_type: string;
  required_tags: string[];
  state: LeaseState;
  created_at: string;
  updated_at: string;
  starts_at: string;
  expires_at: string;
  released_at: string | null;
  failure_message: string | null;
}

export interface LeaseResponse {
  lease: Lease;
  session: Session | null;
}

export interface LeasesResponse {
  leases: LeaseResponse[];
}

export interface AdminLeaseCreateRequest {
  user_id: string;
  board_id: string;
  starts_at: string;
  expires_at: string;
  client_name?: string | null;
}

export interface AdminLeaseUpdateRequest {
  starts_at: string;
  expires_at: string;
  failure_message?: string | null;
}

export interface CreateLeaseRequest {
  board_type: string;
  required_tags?: string[];
  starts_at: string;
  expires_at: string;
}

export interface AdminUserResponse {
  id: string;
  username: string;
  display_name: string;
  nickname: string | null;
  avatar_url: string | null;
  email: string;
  phone: string | null;
  department: string | null;
  title: string | null;
  disabled: boolean;
  status: "active" | "pending" | "rejected";
  last_login_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface AdminUsersResponse {
  users: AdminUserResponse[];
}

export interface AdminUserCreateRequest {
  username: string;
  display_name: string;
  email: string;
  nickname?: string | null;
  avatar_url?: string | null;
  phone?: string | null;
  department?: string | null;
  title?: string | null;
  password: string;
  role_ids: string[];
}

export interface AdminUserUpdateRequest {
  display_name: string;
  email: string;
  nickname?: string | null;
  avatar_url?: string | null;
  phone?: string | null;
  department?: string | null;
  title?: string | null;
  disabled: boolean;
}

export interface AdminPasswordResetRequest {
  password: string;
}

export interface UserPasswordUpdateRequest {
  current_password: string;
  new_password: string;
  confirm_new_password: string;
}

export interface AdminPermissionResponse {
  id: string;
  code: string;
  name: string;
  description: string;
}

export interface AdminPermissionsResponse {
  permissions: AdminPermissionResponse[];
}

export interface AdminRoleResponse {
  id: string;
  name: string;
  display_name: string;
  description: string;
  system: boolean;
  disabled: boolean;
  user_count: number;
  permissions: AdminPermissionResponse[];
  created_at: string;
  updated_at: string;
}

export interface AdminRolesResponse {
  roles: AdminRoleResponse[];
}

export interface AdminRoleCreateRequest {
  name: string;
  display_name: string;
  description: string;
  permission_ids: string[];
}

export interface AdminRoleUpdateRequest {
  display_name: string;
  description: string;
  permission_ids: string[];
}

export interface AdminRoleDisableRequest {
  disabled: boolean;
}

export interface AdminUserRolesResponse {
  roles: AdminRoleResponse[];
}

export interface AdminUserRolesUpdateRequest {
  role_ids: string[];
}

export interface AdminSessionsResponse {
  sessions: AdminSessionResponse[];
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
  dtb_upload_max_mib: number;
}

export interface AdminServerConfigEditable {
  network: TftpNetworkConfig;
  upload_limits: UploadLimitsConfig;
}

export interface AdminServerConfigResponse {
  readonly: AdminServerConfigReadonly;
  editable: AdminServerConfigEditable;
  site: SiteSettingsResponse;
}

export interface UpdateServerConfigRequest {
  editable: AdminServerConfigEditable;
  site: SiteSettingsUpdateRequest;
}

export interface SiteSettingsResponse {
  site_name: string;
  site_subtitle: string;
  logo_url: string | null;
  favicon_url: string | null;
  announcement: string | null;
  maintenance_mode: boolean;
  self_service_enabled: boolean;
  /** `closed` | `auto` | `approval` */
  registration_mode: string;
  default_lease_minutes: number;
  max_lease_minutes: number;
  support_email: string | null;
  support_url: string | null;
  updated_at: string;
}

export type SiteSettingsUpdateRequest = Omit<SiteSettingsResponse, "updated_at">;

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

export interface CreateSessionRequest {
  board_type: string;
  required_tags?: string[];
  client_name?: string | null;
}

export interface SessionCreatedResponse {
  session_id: string;
  board_id: string;
  lease_expires_at: string;
  serial_available: boolean;
  boot_mode: string;
  ws_url: string | null;
}

export interface SessionDetailResponse {
  session: Session;
  board: BoardConfig;
  serial_available: boolean;
  serial_connected: boolean;
  files: FileResponse[];
}
