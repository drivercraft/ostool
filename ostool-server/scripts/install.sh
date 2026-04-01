#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVICE_NAME="ostool-server"
UNIT_FILE="${SCRIPT_DIR}/${SERVICE_NAME}.service"
CONFIG_DIR="/etc/${SERVICE_NAME}"
DATA_DIR="/var/lib/${SERVICE_NAME}"
CONFIG_FILE="${CONFIG_DIR}/config.toml"

LOCAL_PATH=""

usage() {
    echo "Usage: $0 [--local <path>]"
    echo ""
    echo "Install ostool-server as a systemd service."
    echo ""
    echo "Options:"
    echo "  --local <path>  Install from local source directory instead of crates.io"
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --local)
            LOCAL_PATH="$2"
            shift 2
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo "Unknown option: $1"
            usage
            ;;
    esac
done

# --- helper functions ---

prompt_yes_no() {
    local question="$1"
    local default="${2:-Y}"
    local prompt
    if [[ "$default" == "Y" ]]; then
        prompt="[Y/n]"
    else
        prompt="[y/N]"
    fi
    while true; do
        read -rp "${question} ${prompt} " answer
        answer="${answer:-$default}"
        case "$answer" in
            [Yy]|[Yy][Ee][Ss]) return 0 ;;
            [Nn]|[Nn][Oo]) return 1 ;;
            *) echo "Please answer Y or N." ;;
        esac
    done
}

run_cmd() {
    if [[ -n "${SUDO_CMD:-}" ]]; then
        ${SUDO_CMD} "$@"
    else
        "$@"
    fi
}

# --- step 1: check rust environment ---

echo "==> Checking Rust environment..."

if ! command -v rustc &>/dev/null || ! command -v cargo &>/dev/null; then
    echo "Rust is not installed."
    echo ""
    echo "Please install Rust with:"
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    echo ""
    echo "After installation, restart your shell and re-run this script."
    exit 1
fi

echo "Found rustc $(rustc --version), cargo $(cargo --version)"

# --- step 2: determine sudo usage ---

SUDO_CMD=""

if [[ "$(id -u)" -ne 0 ]]; then
    if prompt_yes_no "You are not root. Use sudo for system operations?" "Y"; then
        SUDO_CMD="sudo"
        echo "Will use sudo for system operations."
    else
        echo "Cannot proceed without root privileges. Please re-run with sudo or answer Y."
        exit 1
    fi
fi

# --- step 3: cargo install ---

echo ""
echo "==> Installing ostool-server..."

if [[ -n "$LOCAL_PATH" ]]; then
    LOCAL_PATH="$(cd "$LOCAL_PATH" && pwd)"
    echo "Installing from local source: ${LOCAL_PATH}"
    cargo install --path "${LOCAL_PATH}"
else
    echo "Installing from crates.io..."
    cargo install "${SERVICE_NAME}"
fi

BINDIR="$(dirname "$(command -v ${SERVICE_NAME})")"
echo "Installed to: ${BINDIR}/${SERVICE_NAME}"

# --- step 4: create FHS directories ---

echo ""
echo "==> Creating directories..."

run_cmd mkdir -p "${CONFIG_DIR}"
run_cmd mkdir -p "${DATA_DIR}/boards"
run_cmd mkdir -p "${DATA_DIR}/dtbs"

echo "Created:"
echo "  ${CONFIG_DIR}"
echo "  ${DATA_DIR}/boards"
echo "  ${DATA_DIR}/dtbs"

# --- step 5: generate default config ---

echo ""
echo "==> Checking configuration..."

if run_cmd test -f "${CONFIG_FILE}"; then
    echo "Configuration file already exists: ${CONFIG_FILE}"
    echo "Skipping config generation."
else
    echo "Generating default configuration: ${CONFIG_FILE}"
    run_cmd tee "${CONFIG_FILE}" >/dev/null <<'CONF'
listen_addr = "0.0.0.0:8080"
data_dir = "/var/lib/ostool-server"
board_dir = "/var/lib/ostool-server/boards"
dtb_dir = "/var/lib/ostool-server/dtbs"

[lease]
default_ttl_secs = 900
max_ttl_secs = 3600
gc_interval_secs = 30

[network]
interface = ""

[tftp]
provider = "system_tftpd_hpa"

[tftp.config]
enabled = true
root_dir = "/srv/tftp"
config_path = "/etc/default/tftpd-hpa"
service_name = "tftpd-hpa"
username = "tftp"
address = ":69"
options = "-l -s -c"
manage_config = false
reconcile_on_start = true
CONF
    echo "Default configuration written."
    echo "Please review and edit: ${CONFIG_FILE}"
fi

# --- step 6: install systemd service ---

echo ""
echo "==> Installing systemd service..."

SYSTEMD_UNIT="/etc/systemd/system/${SERVICE_NAME}.service"

# Replace __BINDIR__ placeholder with actual binary path
sed "s|__BINDIR__|${BINDIR}|g" "${UNIT_FILE}" | run_cmd tee "${SYSTEMD_UNIT}" >/dev/null

run_cmd systemctl daemon-reload
run_cmd systemctl enable "${SERVICE_NAME}"

echo "Service installed and enabled."

if prompt_yes_no "Start ${SERVICE_NAME} now?" "Y"; then
    run_cmd systemctl start "${SERVICE_NAME}"
    sleep 1
    run_cmd systemctl status "${SERVICE_NAME}" --no-pager || true
else
    echo "You can start it later with: systemctl start ${SERVICE_NAME}"
fi

echo ""
echo "==> Installation complete!"
echo ""
echo "Useful commands:"
echo "  systemctl status ${SERVICE_NAME}   # Check status"
echo "  systemctl restart ${SERVICE_NAME}  # Restart service"
echo "  journalctl -u ${SERVICE_NAME} -f   # View logs"
echo "  vi ${CONFIG_FILE}                  # Edit config"
