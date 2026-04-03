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
SYSTEM_BIN_DIR="/usr/local/bin"
SYSTEM_BIN_PATH="${SYSTEM_BIN_DIR}/${SERVICE_NAME}"

LOCAL_PATH=""

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
echo "==> Stopping service..."
run_cmd systemctl stop "${SERVICE_NAME}" || true
run_cmd systemctl reset-failed "${SERVICE_NAME}" || true

echo ""
echo "==> Installing updated binary..."

if [[ -n "$LOCAL_PATH" ]]; then
    if [[ ! -d "$LOCAL_PATH" ]]; then
        echo "Local source directory does not exist: ${LOCAL_PATH}" >&2
        exit 1
    fi
    LOCAL_PATH="$(cd "$LOCAL_PATH" && pwd)"
    echo "Upgrading from local source: ${LOCAL_PATH}"
    cargo install --force --path "${LOCAL_PATH}"
else
    echo "Upgrading from crates.io..."
    cargo install --force "${SERVICE_NAME}"
fi

BIN_SOURCE="$(command -v "${SERVICE_NAME}" || true)"
if [[ -z "${BIN_SOURCE}" ]]; then
    echo "Failed to locate upgraded binary: ${SERVICE_NAME}" >&2
    exit 1
fi

BIN_SOURCE="$(readlink -f "${BIN_SOURCE}")"
echo "Cargo installed binary to: ${BIN_SOURCE}"

run_cmd mkdir -p "${SYSTEM_BIN_DIR}"
run_cmd install -m 755 "${BIN_SOURCE}" "${SYSTEM_BIN_PATH}"
echo "Installed updated binary to: ${SYSTEM_BIN_PATH}"

echo ""
echo "==> Refreshing systemd unit..."

SYSTEMD_UNIT="/etc/systemd/system/${SERVICE_NAME}.service"
render_unit_file "${SYSTEM_BIN_PATH}" | run_cmd tee "${SYSTEMD_UNIT}" >/dev/null

run_cmd systemctl daemon-reload
run_cmd systemctl enable "${SERVICE_NAME}"

echo ""
echo "==> Starting upgraded service..."

if run_cmd systemctl start "${SERVICE_NAME}"; then
    sleep 2
    if run_cmd systemctl is-active --quiet "${SERVICE_NAME}"; then
        echo "${SERVICE_NAME} upgrade completed successfully."
        echo ""
        echo "Useful commands:"
        echo "  systemctl status ${SERVICE_NAME}"
        echo "  journalctl -u ${SERVICE_NAME} -f"
        echo "  vi ${CONFIG_FILE}"
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
