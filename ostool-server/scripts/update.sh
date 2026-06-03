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
CONFIG_FILE="${CONFIG_DIR}/config.toml"
DATA_DIR="/var/lib/${SERVICE_NAME}"
HTTP_BOOT_ROOT_DIR="${DATA_DIR}/http-boot"
SYSTEM_BIN_DIR="/usr/local/bin"
SYSTEM_BIN_PATH="${SYSTEM_BIN_DIR}/${SERVICE_NAME}"

LOCAL_PATH=""
STAGING_DIR=""

usage() {
    echo "Usage: $0 [--local <path>]"
    echo ""
    echo "Upgrade an existing ostool-server installation."
    echo ""
    echo "Options:"
    echo "  --local <path>  Upgrade from local source directory instead of crates.io"
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
    echo "If the Web UI was already open during upgrade, refresh the page after the service restarts."
}

cleanup() {
    if [[ -n "${STAGING_DIR}" && -d "${STAGING_DIR}" ]]; then
        rm -rf "${STAGING_DIR}"
    fi
}

trap cleanup EXIT

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

echo ""
echo "==> Checking current installation..."

if ! run_cmd test -f "${SYSTEM_BIN_PATH}"; then
    echo "Existing binary not found: ${SYSTEM_BIN_PATH}"
    echo "Run install.sh first, then use update.sh for upgrades."
    exit 1
fi

if ! run_cmd systemctl cat "${SERVICE_NAME}" >/dev/null 2>&1; then
    echo "Systemd service ${SERVICE_NAME} is not installed."
    echo "Run install.sh first, then use update.sh for upgrades."
    exit 1
fi

if run_cmd test -f "${CONFIG_FILE}"; then
    echo "Will preserve existing config: ${CONFIG_FILE}"
else
    echo "Config file not found: ${CONFIG_FILE}"
    echo "The upgraded service will recreate defaults on first start."
fi

echo "Will preserve data directory: ${DATA_DIR}"

echo ""
echo "==> Ensuring HTTP Boot data directories..."
run_cmd mkdir -p "${HTTP_BOOT_ROOT_DIR}"
echo "Ensured HTTP Boot root directory: ${HTTP_BOOT_ROOT_DIR}"

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

echo ""
echo "==> Building updated binary..."

STAGING_DIR="$(mktemp -d)"
STAGED_BIN_PATH="${STAGING_DIR}/bin/${SERVICE_NAME}"

if [[ -n "$LOCAL_PATH" ]]; then
    if [[ ! -d "$LOCAL_PATH" ]]; then
        echo "Local source directory does not exist: ${LOCAL_PATH}" >&2
        exit 1
    fi
    LOCAL_PATH="$(cd "$LOCAL_PATH" && pwd)"
    echo "Upgrading from local source: ${LOCAL_PATH}"
    cargo install --root "${STAGING_DIR}" --path "${LOCAL_PATH}"
else
    echo "Upgrading from crates.io..."
    cargo install --root "${STAGING_DIR}" "${SERVICE_NAME}"
fi

if [[ ! -x "${STAGED_BIN_PATH}" ]]; then
    echo "Failed to locate staged binary: ${STAGED_BIN_PATH}" >&2
    exit 1
fi

echo "Built staged binary: ${STAGED_BIN_PATH}"

echo ""
echo "==> Refreshing systemd unit..."

SYSTEMD_UNIT="/etc/systemd/system/${SERVICE_NAME}.service"
render_unit_file "${SYSTEM_BIN_PATH}" | run_cmd tee "${SYSTEMD_UNIT}" >/dev/null

run_cmd systemctl daemon-reload
run_cmd systemctl enable "${SERVICE_NAME}"

echo ""
echo "==> Replacing binary and restarting service..."

run_cmd systemctl stop "${SERVICE_NAME}" || true
run_cmd systemctl reset-failed "${SERVICE_NAME}" || true
run_cmd mkdir -p "${SYSTEM_BIN_DIR}"
run_cmd install -m 755 "${STAGED_BIN_PATH}" "${SYSTEM_BIN_PATH}"
echo "Installed updated binary to: ${SYSTEM_BIN_PATH}"

if run_cmd systemctl start "${SERVICE_NAME}"; then
    sleep 2
    if run_cmd systemctl is-active --quiet "${SERVICE_NAME}"; then
        echo "${SERVICE_NAME} upgrade completed successfully."
        echo ""
        echo "Useful commands:"
        echo "  systemctl status ${SERVICE_NAME}"
        echo "  journalctl -u ${SERVICE_NAME} -f"
        echo "  vi ${CONFIG_FILE}"
        echo ""
        print_web_ui_hint
    else
        echo "${SERVICE_NAME} failed to become active. Recent logs:"
        run_cmd journalctl -u "${SERVICE_NAME}" -n 50 --no-pager || true
        exit 1
    fi
else
    echo "Failed to start ${SERVICE_NAME}. Recent logs:"
    run_cmd journalctl -u "${SERVICE_NAME}" -n 50 --no-pager || true
    exit 1
fi
