#!/usr/bin/env bash
set -Eeuo pipefail

# 仅管理仓库的 Docker Compose 环境，不修改 systemd 或 /etc/ostool-server。
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
export OSTOOL_ENV_FILE=${OSTOOL_ENV_FILE:-"$SCRIPT_DIR/../.env"}

case "${1:-start}" in
    help|-h|--help)
        echo "用法：$0 {start|stop|restart|build|status|logs|seed|cert}"
        echo "start/restart：构建并启动 release 容器；stop：关闭容器，保留数据。"
        echo "seed：独立导入演示数据并重启；cert：导出 IP HTTPS 使用的根证书。"
        exit 0
        ;;
esac
command -v docker >/dev/null || { echo "需要安装 Docker 和 Compose 插件" >&2; exit 1; }
[[ -f "$OSTOOL_ENV_FILE" ]] || { echo "请先复制 ostool-server/.env.example 为 .env 并填写配置" >&2; exit 1; }
OSTOOL_ENV_FILE="$(cd -- "$(dirname -- "$OSTOOL_ENV_FILE")" && pwd)/$(basename -- "$OSTOOL_ENV_FILE")"
COMPOSE=(docker compose --project-name ostool-server --env-file "$OSTOOL_ENV_FILE" -f "$SCRIPT_DIR/../deploy/compose.yaml")

case "${1:-start}" in
    start|restart)
        RECREATE=()
        if [[ "${1:-start}" == restart ]]; then RECREATE=(--force-recreate); fi
        "${COMPOSE[@]}" up -d --build "${RECREATE[@]}" --wait --wait-timeout 120
        echo "环境已启动；访问地址见 .env 的 OSTOOL_PUBLIC_BASE_URL。"
        echo "IP HTTPS 首次使用请运行 $0 cert，并在访问设备上信任根证书。"
        ;;
    stop) "${COMPOSE[@]}" down ;;
    build) "${COMPOSE[@]}" build server ;;
    status) "${COMPOSE[@]}" ps -a ;;
    logs) "${COMPOSE[@]}" logs --tail 100 "${@:2}" ;;
    seed)
        "${COMPOSE[@]}" up -d --wait postgres redis
        "${COMPOSE[@]}" stop caddy server
        "${COMPOSE[@]}" run --build --rm --no-deps --entrypoint ostool-server-seed-demo server \
            --config /etc/ostool-server/config.toml --env-file /dev/null
        "${COMPOSE[@]}" up -d --wait
        ;;
    cert)
        CERT_DIR="$SCRIPT_DIR/../../.ostool-server-dev"
        mkdir -p "$CERT_DIR"
        "${COMPOSE[@]}" cp caddy:/data/caddy/pki/authorities/local/root.crt "$CERT_DIR/root.crt"
        echo "请在访问设备上导入并信任：$(cd "$CERT_DIR" && pwd)/root.crt"
        ;;
    *) echo "未知命令：$1，请使用 --help" >&2; exit 2 ;;
esac
