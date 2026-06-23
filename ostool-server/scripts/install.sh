#!/usr/bin/env bash
set -euo pipefail

SCRIPT_SOURCE="${BASH_SOURCE[0]:-}"
if [[ -n "${SCRIPT_SOURCE}" && -f "${SCRIPT_SOURCE}" ]]; then
    SCRIPT_DIR="$(cd "$(dirname "${SCRIPT_SOURCE}")" && pwd)"
else
    SCRIPT_DIR=""
fi

SERVICE_NAME="ostool-server"
UNIT_FILE=""
if [[ -n "${SCRIPT_DIR}" ]]; then
    UNIT_FILE="${SCRIPT_DIR}/${SERVICE_NAME}.service"
fi
CONFIG_DIR="/etc/${SERVICE_NAME}"
DATA_DIR="/var/lib/${SERVICE_NAME}"
CONFIG_FILE="${CONFIG_DIR}/config.toml"
TFTP_ROOT_DIR="/srv/tftp"
SYSTEM_BIN_DIR="/usr/local/bin"
SYSTEM_BIN_PATH="${SYSTEM_BIN_DIR}/${SERVICE_NAME}"

LOCAL_PATH=""
STAGING_DIR=""

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
            if [[ $# -lt 2 ]]; then
                echo "Missing argument for --local"
                usage
            fi
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

load_unit_template() {
    if [[ -n "${UNIT_FILE}" && -f "${UNIT_FILE}" ]]; then
        cat "${UNIT_FILE}"
        return 0
    fi

    cat <<'EOF'
[Unit]
Description=OSTool Board Server
After=network.target

[Service]
Type=simple
User=root
Group=root
ExecStart=__BIN_PATH__ --config /etc/ostool-server/config.toml
Restart=on-failure
RestartSec=5
WorkingDirectory=/var/lib/ostool-server

PrivateTmp=true

StandardOutput=journal
StandardError=journal
SyslogIdentifier=ostool-server

[Install]
WantedBy=multi-user.target
EOF
}

render_unit_file() {
    local bin_path="$1"
    load_unit_template | sed "s|__BIN_PATH__|${bin_path}|g"
}

install_tftpd_hpa() {
    if command -v apt-get &>/dev/null; then
        run_cmd apt-get install -y tftpd-hpa
        return
    fi

    echo "Automatic tftpd-hpa installation is only supported on apt-based systems." >&2
    echo "Please install tftpd-hpa manually, then re-run this script." >&2
    exit 1
}

print_web_ui_hint() {
    local host_ip
    host_ip="$(hostname -I 2>/dev/null | awk '{print $1}')"
    if [[ -n "${host_ip}" ]]; then
        echo "Web UI: http://${host_ip}:2999/admin/"
    else
        echo "Web UI: http://<server-ip>:2999/admin/"
    fi
    echo "If the Web UI was already open during installation, refresh the page after the service restarts."
}

cleanup() {
    if [[ -n "${STAGING_DIR}" && -d "${STAGING_DIR}" ]]; then
        rm -rf "${STAGING_DIR}"
    fi
}

trap cleanup EXIT

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

# --- step 1b: check web UI build environment ---

echo ""
echo "==> Checking web UI build environment..."

if ! command -v node &>/dev/null; then
    echo "Node.js is not installed."
    echo ""
    echo "ostool-server embeds the web UI at build time, and cargo install runs pnpm."
    echo "Please install Node.js 18 or newer, then re-run this script."
    exit 1
fi

NODE_MAJOR="$(node -p 'process.versions.node.split(".")[0]' 2>/dev/null || echo 0)"
if [[ "${NODE_MAJOR}" -lt 18 ]]; then
    echo "Node.js $(node --version) is too old; ostool-server web UI requires Node.js 18 or newer."
    exit 1
fi

if ! command -v pnpm &>/dev/null; then
    if command -v corepack &>/dev/null; then
        echo "pnpm is missing; enabling pnpm through corepack..."
        corepack enable
        corepack prepare pnpm@10.33.0 --activate
    else
        echo "pnpm is not installed and corepack is unavailable."
        echo ""
        echo "Install pnpm with one of:"
        echo "  corepack enable && corepack prepare pnpm@10.33.0 --activate"
        echo "  npm install -g pnpm"
        exit 1
    fi
fi

echo "Found node $(node --version), pnpm $(pnpm --version)"

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

# --- step 3: stop existing service ---

echo ""
echo "==> Checking existing service..."

if run_cmd systemctl cat "${SERVICE_NAME}" >/dev/null 2>&1; then
    echo "Stopping existing ${SERVICE_NAME} service..."
    run_cmd systemctl stop "${SERVICE_NAME}" || true
    run_cmd systemctl reset-failed "${SERVICE_NAME}" || true
    run_cmd systemctl daemon-reload || true
    echo "Existing service stopped."
else
    echo "No existing ${SERVICE_NAME} service found."
fi

# --- step 3b: check system TFTP dependency ---

echo ""
echo "==> Checking system TFTP dependency..."

if command -v in.tftpd &>/dev/null; then
    echo "Found in.tftpd: $(command -v in.tftpd)"
else
    echo "tftpd-hpa is missing; installing it now..."
    install_tftpd_hpa
    if ! command -v in.tftpd &>/dev/null; then
        echo "tftpd-hpa installation completed but in.tftpd is still not in PATH." >&2
        exit 1
    fi
    echo "Found in.tftpd: $(command -v in.tftpd)"
fi

# --- step 4: clean previous configuration ---

echo ""
echo "==> Cleaning previous configuration..."

if run_cmd test -d "${CONFIG_DIR}"; then
    echo "Removing existing configuration directory: ${CONFIG_DIR}"
    run_cmd rm -rf "${CONFIG_DIR}"
else
    echo "No existing configuration directory found."
fi

run_cmd mkdir -p "${CONFIG_DIR}"
echo "Prepared configuration directory: ${CONFIG_DIR}"

# --- step 5: cargo install ---

echo ""
echo "==> Installing ostool-server..."

STAGING_DIR="$(mktemp -d)"
STAGED_BIN_PATH="${STAGING_DIR}/bin/${SERVICE_NAME}"

if [[ -n "$LOCAL_PATH" ]]; then
    if [[ ! -d "$LOCAL_PATH" ]]; then
        echo "Local source directory does not exist: ${LOCAL_PATH}" >&2
        exit 1
    fi
    LOCAL_PATH="$(cd "$LOCAL_PATH" && pwd)"
    echo "Installing from local source: ${LOCAL_PATH}"
    cargo install --root "${STAGING_DIR}" --path "${LOCAL_PATH}"
else
    echo "Installing from crates.io..."
    cargo install --root "${STAGING_DIR}" "${SERVICE_NAME}"
fi

if [[ ! -x "${STAGED_BIN_PATH}" ]]; then
    echo "Failed to locate staged binary: ${STAGED_BIN_PATH}" >&2
    exit 1
fi

echo "Built staged binary: ${STAGED_BIN_PATH}"

echo ""
echo "==> Installing system binary..."
run_cmd mkdir -p "${SYSTEM_BIN_DIR}"
run_cmd install -m 755 "${STAGED_BIN_PATH}" "${SYSTEM_BIN_PATH}"
echo "Installed system binary to: ${SYSTEM_BIN_PATH}"

# --- step 6: create FHS directories ---

echo ""
echo "==> Creating directories..."

run_cmd mkdir -p "${CONFIG_DIR}"
run_cmd mkdir -p "${DATA_DIR}"
run_cmd mkdir -p "${DATA_DIR}/boards"
run_cmd mkdir -p "${DATA_DIR}/dtbs"
run_cmd mkdir -p "${TFTP_ROOT_DIR}"

echo "Created:"
echo "  ${CONFIG_DIR}"
echo "  ${DATA_DIR}/boards"
echo "  ${DATA_DIR}/dtbs"
echo "  ${TFTP_ROOT_DIR}"
echo "Note: the generated config uses MySQL by default; configure MySQL separately or switch [database] to sqlite."

# --- step 7: generate default config ---

echo ""
echo "==> Checking configuration..."

if run_cmd test -f "${CONFIG_FILE}"; then
    echo "Configuration file already exists: ${CONFIG_FILE}"
else
    echo "Configuration file will be created automatically on first start: ${CONFIG_FILE}"
fi
echo "After first install, create an administrator account with:"
echo "  ${SYSTEM_BIN_PATH} --config ${CONFIG_FILE} admin init --username admin --password '<change-me>'"

# --- step 8: install systemd service ---

echo ""
echo "==> Installing systemd service..."

SYSTEMD_UNIT="/etc/systemd/system/${SERVICE_NAME}.service"

# Replace __BIN_PATH__ placeholder with actual binary path
render_unit_file "${SYSTEM_BIN_PATH}" | run_cmd tee "${SYSTEMD_UNIT}" >/dev/null

run_cmd systemctl daemon-reload
run_cmd systemctl enable "${SERVICE_NAME}"

echo "Service installed and enabled."

if prompt_yes_no "Start ${SERVICE_NAME} now?" "Y"; then
    run_cmd systemctl reset-failed "${SERVICE_NAME}" || true
    if run_cmd systemctl start "${SERVICE_NAME}"; then
        sleep 2
        if run_cmd systemctl is-active --quiet "${SERVICE_NAME}"; then
            run_cmd systemctl status "${SERVICE_NAME}" --no-pager || true
        else
            echo "Failed to bring ${SERVICE_NAME} to an active state."
            run_cmd systemctl status "${SERVICE_NAME}" --no-pager || true
            echo "Recent logs:"
            run_cmd journalctl -u "${SERVICE_NAME}" -n 20 --no-pager || true
            exit 1
        fi
    else
        echo "Failed to start ${SERVICE_NAME}."
        echo "Recent logs:"
        run_cmd journalctl -u "${SERVICE_NAME}" -n 20 --no-pager || true
        exit 1
    fi
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
echo ""
print_web_ui_hint
