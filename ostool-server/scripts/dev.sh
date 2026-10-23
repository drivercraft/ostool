#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
SERVER_DIR=$(cd -- "$SCRIPT_DIR/.." && pwd -P)
REPO_DIR=$(cd -- "$SERVER_DIR/.." && pwd -P)
ENV_FILE=${OSTOOL_ENV_FILE:-"$SERVER_DIR/.env"}
ACTION= LOG_SERVICE= EDITION=local
COMPOSE=()
COMPOSE_FILES=()
ALL_COMPOSE=()
BOLD= RESET= BLUE=
if [[ -t 1 && ${TERM:-} != dumb && -z ${NO_COLOR+x} ]]; then
    BOLD=$'\033[1m'; BLUE=$'\033[36m'; RESET=$'\033[0m'
fi

dev_usage() {
    printf '%sOSTool Server · Docker 调试环境%s\n\n' "$BOLD" "$RESET"
    cat <<'EOF'
用法：
  ostool-server/scripts/dev.sh <命令> [选项]

命令：
  start                     构建 release 镜像并启动环境
  stop                      关闭本环境的容器，保留数据卷
  restart                   重新构建并重启后端，应用最新代码
  build                     仅构建 release 镜像
  check                     检查 Compose 与访问配置，不启动容器
  status                    显示容器状态
  logs [服务]               查看最近日志，可指定 server、postgres、redis、caddy
  seed                      独立导入预置业务数据，然后重启后端
  cert                      导出 public 模式内部 CA 根证书
  help, -h, --help          显示使用说明

选项：
  --env-file <路径>         唯一启动配置，默认 ostool-server/.env
  --compose-file <路径>     Docker 叠加配置，可重复；真实串口使用 deploy/compose.serial.yaml

示例：
  ostool-server/scripts/dev.sh start
  ostool-server/scripts/dev.sh restart
  ostool-server/scripts/dev.sh logs server
  ostool-server/scripts/dev.sh start --env-file /path/to/.env

说明：
  仅管理 Docker，不调用 deploy.sh，不操作宿主机 systemd 或 /etc。
  local 直接使用 HTTP；public 启动标准 Caddy 容器提供 HTTPS。
  使用与正式部署相同的 release 程序、数据库和鉴权；预置业务数据仅由 seed 导入。
  与 deploy.sh docker 使用同一 Compose 项目及数据卷，不要同时管理同一环境。
EOF
}

dev_parse_args() {
    if (($# == 0)); then dev_usage; exit 0; fi
    ACTION=$1; shift
    case $ACTION in
        help|-h|--help) dev_usage; exit 0;;
        start|stop|restart|build|check|status|logs|seed|cert) ;;
        *) printf '[错误] 未知命令：%s；使用 --help 查看说明。\n' "$ACTION" >&2; exit 2;;
    esac
    while (($#)); do
        case $1 in
            --env-file|--compose-file)
                [[ $# -ge 2 && -n $2 && $2 != --* ]] || { printf '[错误] %s 缺少路径。\n' "$1" >&2; exit 2; }
                if [[ $1 == --env-file ]]; then ENV_FILE=$2; else COMPOSE_FILES+=("$2"); fi
                shift 2;;
            -h|--help) dev_usage; exit 0;;
            *)
                if [[ $ACTION == logs && -z $LOG_SERVICE && $1 != -* ]]; then LOG_SERVICE=$1; shift;
                else printf '[错误] 未知参数：%s\n' "$1" >&2; exit 2; fi;;
        esac
    done
    if [[ -n $LOG_SERVICE ]]; then
        case $LOG_SERVICE in server|postgres|redis|caddy) ;; *) printf '[错误] 未知服务：%s\n' "$LOG_SERVICE" >&2; exit 2;; esac
    fi
}

dev_prepare() {
    command -v docker >/dev/null || { printf '[错误] 请安装 Docker 和 Compose 插件。\n' >&2; exit 1; }
    [[ -f $ENV_FILE ]] || { printf '[错误] 配置不存在：%s\n请复制 ostool-server/.env.example 为 .env 并填写配置。\n' "$ENV_FILE" >&2; exit 1; }
    ENV_FILE=$(cd -- "$(dirname -- "$ENV_FILE")" && pwd -P)/$(basename -- "$ENV_FILE")
    # 只读取选定文件，不继承终端中遗留的应用配置；不执行 dotenv 中的 Shell 内容。
    for name in ${!OSTOOL_@}; do unset "$name"; done
    export OSTOOL_ENV_FILE=$ENV_FILE
    # profile 由当前入口选择，不继承其他 Compose 项目的终端设置。
    export COMPOSE_PROFILES=
    COMPOSE=(docker compose --project-name ostool-server --env-file "$ENV_FILE" -f "$SERVER_DIR/deploy/compose.yaml")
    for file in "${COMPOSE_FILES[@]}"; do COMPOSE+=(-f "$file"); done
    ALL_COMPOSE=("${COMPOSE[@]}" --profile gateway)
    local resolved line edition=local origin= allowed= listener=127.0.0.1:2999 site
    local sites=()
    resolved=$("${COMPOSE[@]}" config --environment)
    while IFS= read -r line; do
        case $line in
            OSTOOL_EDITION=*) edition=${line#*=};;
            OSTOOL_PUBLIC_BASE_URL=*) origin=${line#*=};;
            OSTOOL_ALLOWED_ORIGINS=*) allowed=${line#*=};;
            OSTOOL_LISTEN_ADDR=*) listener=${line#*=};;
        esac
    done <<< "$resolved"
    EDITION=$edition
    case $edition in
        public)
            if [[ $ACTION == start || $ACTION == restart || $ACTION == check || $ACTION == seed ]]; then
                [[ $origin == https://* && $listener =~ ^127\.0\.0\.1:[0-9]+$ ]] || { printf '[错误] public 模式需要 HTTPS 主站地址和 127.0.0.1 API 监听地址。\n' >&2; exit 1; }
            fi
            OSTOOL_GATEWAY_SITES=$origin
            read -r -a sites <<< "${allowed//,/ }"
            for site in "${sites[@]}"; do
                if [[ " $OSTOOL_GATEWAY_SITES " != *" $site "* ]]; then OSTOOL_GATEWAY_SITES+=" $site"; fi
            done
            export OSTOOL_GATEWAY_SITES
            export OSTOOL_GATEWAY_UPSTREAM=$listener
            COMPOSE+=(--profile gateway);;
        local) ;;
        *) printf '[错误] OSTOOL_EDITION 只能是 local 或 public。\n' >&2; exit 1;;
    esac
    printf '%s[环境]%s %s · %s\n' "$BLUE" "$RESET" "$edition" "$ENV_FILE"
}

dev_run() {
    case $ACTION in
        start|restart)
            # local 模式停用本项目的网关，避免旧公网入口代理免登录后台。
            if [[ $EDITION == local ]]; then
                "${ALL_COMPOSE[@]}" stop caddy
            fi
            if [[ $ACTION == restart ]]; then
                "${COMPOSE[@]}" up -d --build --force-recreate --wait --wait-timeout 120 server
                "${COMPOSE[@]}" up -d --wait --wait-timeout 120
            else
                "${COMPOSE[@]}" up -d --build --wait --wait-timeout 120
            fi
            if [[ $EDITION == public ]]; then
                "${COMPOSE[@]}" exec -T caddy caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile
            fi
            printf '[完成] 环境已启动；访问地址使用 .env 中的 OSTOOL_PUBLIC_BASE_URL。\n';;
        stop) "${ALL_COMPOSE[@]}" down;;
        build) "${COMPOSE[@]}" build server;;
        check) "${COMPOSE[@]}" config --quiet; printf '[完成] 配置检查通过，未启动容器。\n';;
        status) "${ALL_COMPOSE[@]}" ps -a;;
        logs) "${ALL_COMPOSE[@]}" logs --tail 100 ${LOG_SERVICE:+"$LOG_SERVICE"};;
        seed)
            "${COMPOSE[@]}" up -d --wait postgres redis
            "${COMPOSE[@]}" stop server
            "${COMPOSE[@]}" run --build --rm --no-deps --entrypoint ostool-server-seed-demo server --env-file /dev/null
            "${COMPOSE[@]}" up -d --wait
            printf '[完成] 预置业务数据已导入，已有记录保留。\n';;
        cert)
            [[ $EDITION == public ]] || { printf '[信息] local 使用 HTTP，无需证书。\n'; return; }
            mkdir -p "$REPO_DIR/.ostool-server-dev"
            "${ALL_COMPOSE[@]}" cp caddy:/data/caddy/pki/authorities/local/root.crt "$REPO_DIR/.ostool-server-dev/root.crt"
            printf '[完成] 根证书：%s/.ostool-server-dev/root.crt\n请在访问设备上信任此证书。\n' "$REPO_DIR";;
    esac
}

main() {
    dev_parse_args "$@"
    dev_prepare
    dev_run
}

main "$@"
