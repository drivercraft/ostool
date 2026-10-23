#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
SERVER_DIR=$(cd -- "$SCRIPT_DIR/.." && pwd -P)
REPO_DIR=$(cd -- "$SERVER_DIR/.." && pwd -P)
MODE= ACTION= EDITION=local
ENV_FILE=${OSTOOL_ENV_FILE:-"$SERVER_DIR/.env"}
DRY_RUN=false OUTPUT_DIR=
COMPOSE=()
BOLD= RESET= BLUE=
if [[ -t 1 && ${TERM:-} != dumb && -z ${NO_COLOR+x} ]]; then
    BOLD=$'\033[1m'; BLUE=$'\033[36m'; RESET=$'\033[0m'
fi

deploy_usage() {
    printf '%sOSTool Server · 部署与更新%s\n\n' "$BOLD" "$RESET"
    cat <<'EOF'
用法：
  ostool-server/scripts/deploy.sh <部署方式> <命令> [选项]

部署方式：
  docker                    使用 release 容器和内置 PostgreSQL、Redis
  systemd                   安装本机服务，使用已配置的 PostgreSQL、Redis

命令：
  up                        构建并部署；再次执行用于更新，保留配置备份与数据
  down                      停止本项目服务，保留数据和现有共享 Caddy
  restart                   重新构建并重启服务
  build                     仅构建可部署的 release 产物
  check                     检查部署配置，不安装或启动服务
  status                    显示服务状态
  logs                      查看最近日志
  help, -h, --help          显示使用说明

选项：
  --env-file <路径>         唯一启动配置，默认 ostool-server/.env
  --compose-file <路径>     Docker 叠加配置，可重复；真实串口使用 deploy/compose.serial.yaml
  --dry-run                 只显示计划，不构建、不安装、不重启
  --output-dir <目录>       systemd up/restart 专用，只生成安装文件

示例：
  ostool-server/scripts/deploy.sh docker up
  ostool-server/scripts/deploy.sh docker up --dry-run
  ostool-server/scripts/deploy.sh systemd up --env-file /path/to/.env
  ostool-server/scripts/deploy.sh systemd up --output-dir /tmp/ostool-preview

说明：
  首次使用请复制 .env.example 为 .env，填写连接地址和必要密钥。
  public 接入现有标准 Caddy；仅更新本站点片段，验证后 reload，不接管其他网站。
  本地 Docker 调试、演示数据与内部 CA 证书导出使用 dev.sh。
EOF
}

COMPOSE_FILES=()
deploy_parse_args() {
    if (($# == 0)); then deploy_usage; exit 0; fi
    case $1 in help|-h|--help) deploy_usage; exit 0;; esac
    (($# >= 2)) || { printf '[错误] 请同时指定部署方式和命令。\n' >&2; exit 2; }
    MODE=$1 ACTION=$2; shift 2
    case $ACTION in help|-h|--help) deploy_usage; exit 0;; esac
    case $MODE in docker|systemd) ;; *) printf '[错误] 未知部署方式：%s\n' "$MODE" >&2; exit 2;; esac
    case $ACTION in up|down|restart|build|status|logs|check) ;; *) printf '[错误] 未知命令：%s\n' "$ACTION" >&2; exit 2;; esac
    while (($#)); do
        case $1 in
            --env-file|--output-dir|--compose-file)
                [[ $# -ge 2 && -n $2 && $2 != --* ]] || { printf '[错误] %s 缺少路径。\n' "$1" >&2; exit 2; }
                if [[ $1 == --env-file ]]; then ENV_FILE=$2; elif [[ $1 == --output-dir ]]; then OUTPUT_DIR=$2; else COMPOSE_FILES+=("$2"); fi
                shift 2;;
            --dry-run) DRY_RUN=true; shift;;
            -h|--help) deploy_usage; exit 0;;
            *) printf '[错误] 未知参数：%s\n' "$1" >&2; exit 2;;
        esac
    done
    if [[ -n $OUTPUT_DIR && ( $MODE != systemd || ( $ACTION != up && $ACTION != restart ) ) ]]; then
        printf '[错误] --output-dir 只用于 systemd up/restart。\n' >&2; exit 2
    fi
    if [[ $MODE != docker && ${#COMPOSE_FILES[@]} -gt 0 ]]; then printf '[错误] --compose-file 只用于 Docker 部署。\n' >&2; exit 2; fi
}

deploy_setting() {
    local key=$1 fallback=${2:-}
    if [[ $MODE == docker ]]; then
        "${COMPOSE[@]}" config --environment | awk -v key="$key" -v fallback="$fallback" '
            index($0,key"=")==1 {value=substr($0,length(key)+2); found=1} END {if(found)printf "%s",value;else printf "%s",fallback}'
    else
        node --input-type=commonjs - "$ENV_FILE" "$key" "$fallback" <<'JS'
const {readFileSync}=require('node:fs');
const {parseEnv}=require('node:util');
const [, ,file,key,fallback]=process.argv;
const value=parseEnv(readFileSync(file,'utf8'))[key] ?? fallback;
if(/[\r\n\0]/.test(value)) throw Error('部署参数不能包含换行');
process.stdout.write(value);
JS
    fi
}
deploy_prepare() {
    if [[ $MODE == systemd && ( $ACTION == status || $ACTION == logs || $ACTION == down || $ACTION == build ) ]]; then return; fi
    [[ -f $ENV_FILE ]] || { printf '[错误] 配置不存在：%s\n请复制 ostool-server/.env.example 为 .env。\n' "$ENV_FILE" >&2; exit 1; }
    ENV_FILE=$(cd -- "$(dirname -- "$ENV_FILE")" && pwd -P)/$(basename -- "$ENV_FILE")
    for name in ${!OSTOOL_@}; do unset "$name"; done
    export OSTOOL_ENV_FILE=$ENV_FILE
    # profile 由当前入口选择，不继承其他 Compose 项目的终端设置。
    export COMPOSE_PROFILES=
    COMPOSE=(docker compose --project-name ostool-server --env-file "$ENV_FILE" -f "$SERVER_DIR/deploy/compose.yaml")
    for file in "${COMPOSE_FILES[@]}"; do COMPOSE+=(-f "$file"); done
    if [[ $MODE == docker ]]; then
        command -v docker >/dev/null && docker compose version >/dev/null || { printf '[错误] 需要 Docker 和 Compose 插件。\n' >&2; exit 1; }
    else
        command -v node >/dev/null && node -e 'if(!require("node:util").parseEnv)process.exit(1)' || { printf '[错误] 需要支持 util.parseEnv 的 Node.js（20.12+）。\n' >&2; exit 1; }
    fi
    EDITION=$(deploy_setting OSTOOL_EDITION local)
    [[ $EDITION == local || $EDITION == public ]] || { printf '[错误] OSTOOL_EDITION 只能是 local 或 public。\n' >&2; exit 1; }
    if [[ $EDITION == public && $ACTION =~ ^(up|restart|check)$ ]]; then
        [[ $(deploy_setting OSTOOL_LISTEN_ADDR 127.0.0.1:2999) =~ ^127\.0\.0\.1:[0-9]+$ ]] || { printf '[错误] public API 必须绑定 127.0.0.1，由 Caddy 提供 HTTPS。\n' >&2; exit 1; }
        [[ $(deploy_setting OSTOOL_PUBLIC_BASE_URL) == https://* ]] || { printf '[错误] public 必须配置 HTTPS 主站地址。\n' >&2; exit 1; }
    fi
    printf '%s[部署]%s %s · %s · %s\n' "$BLUE" "$RESET" "$MODE" "$EDITION" "$ENV_FILE"
}

deploy_exec() {
    if $DRY_RUN; then printf '计划：'; printf ' %q' "$@"; printf '\n'; else "$@"; fi
}

deploy_docker() {
    case $ACTION in
        up) deploy_exec "${COMPOSE[@]}" up -d --build --wait --wait-timeout 120;;
        restart)
            deploy_exec "${COMPOSE[@]}" up -d --build --force-recreate --wait --wait-timeout 120 server
            deploy_exec "${COMPOSE[@]}" up -d --wait --wait-timeout 120;;
        down) deploy_exec "${COMPOSE[@]}" down;;
        build) deploy_exec "${COMPOSE[@]}" build server;;
        check) deploy_exec "${COMPOSE[@]}" config --quiet;;
        status) deploy_exec "${COMPOSE[@]}" ps -a;;
        logs) deploy_exec "${COMPOSE[@]}" logs --tail 100;;
    esac
}

deploy_systemd() {
    if [[ $ACTION == build ]]; then
        if $DRY_RUN; then printf '计划：cargo build --locked --release -p ostool-server --bins\n';
        else (cd "$REPO_DIR" && cargo build --locked --release -p ostool-server --bins); fi
        return
    fi
    # 服务查看和停止不需要构建工具，也不要求原配置文件仍存在。
    if [[ $ACTION == status ]]; then
        if $DRY_RUN; then echo '计划：systemctl status ostool-server'; else systemctl status ostool-server --no-pager; fi
        return
    fi
    if [[ $ACTION == logs ]]; then
        if $DRY_RUN; then echo '计划：journalctl -u ostool-server'; else journalctl -u ostool-server -n 100 --no-pager; fi
        return
    fi
    if [[ $ACTION == down ]]; then
        if $DRY_RUN; then echo '计划：systemctl disable --now ostool-server';
        elif [[ $(id -u) == 0 ]]; then systemctl disable --now ostool-server;
        else sudo systemctl disable --now ostool-server; fi
        return
    fi
    # 仅下列安装动作需要提权；check、dry-run 和输出到临时目录不会修改系统。
    if $DRY_RUN; then
        echo "计划：systemd $ACTION（$EDITION），配置 $ENV_FILE；保留现有数据。"
        if [[ $EDITION == public && ( $ACTION == up || $ACTION == restart ) ]]; then echo '计划：仅更新本站点片段，验证整份 Caddyfile 后 reload 默认 caddy.service，保留其他站点。'; fi
        exit 0
    fi
    if [[ $ACTION == check ]]; then
        [[ -n $(deploy_setting OSTOOL_DATABASE_URL) ]] || { echo '缺少 OSTOOL_DATABASE_URL' >&2; exit 1; }
        echo "配置检查通过：systemd / $EDITION（未连接数据库，未安装服务）"; exit 0
    fi
    STAGING_DIR=$(mktemp -d)
    trap 'find "$STAGING_DIR" -maxdepth 1 -type f -delete; rmdir -- "$STAGING_DIR"' EXIT
    if [[ -n $OUTPUT_DIR || $ACTION == up || $ACTION == restart ]]; then
        install -m 600 "$ENV_FILE" "$STAGING_DIR/.env"
        install -m 644 "$SERVER_DIR/deploy/ostool-server.service" "$STAGING_DIR/ostool-server.service"
        install -m 644 "$SERVER_DIR/deploy/tftpd.readonly.rules" "$STAGING_DIR/ostool-server-tftp.rules"
        if [[ $EDITION == public ]]; then
            install -m 644 "$SERVER_DIR/deploy/ostool-server.caddy" "$STAGING_DIR/ostool-server.caddy"
            node --input-type=commonjs - "$ENV_FILE" "$STAGING_DIR/ostool-server.caddy" <<'JS'
const fs=require('node:fs'),{parseEnv}=require('node:util');
const env=parseEnv(fs.readFileSync(process.argv[2],'utf8'));
const origin=new URL(env.OSTOOL_PUBLIC_BASE_URL);
if(origin.protocol!=='https:' || origin.pathname!=='/' || origin.username || origin.password || origin.search || origin.hash)throw Error('主站地址必须是 HTTPS 来源');
const upstream=env.OSTOOL_LISTEN_ADDR || '127.0.0.1:2999';
if(!/^127\.0\.0\.1:[0-9]+$/.test(upstream))throw Error('公网 API 必须使用回环地址');
fs.writeFileSync(process.argv[3],fs.readFileSync(process.argv[3],'utf8').replace('boards.example.com',origin.origin).replace('127.0.0.1:2999',upstream));
JS
        fi
        if [[ -n $OUTPUT_DIR ]]; then
            mkdir -p "$OUTPUT_DIR"
            cp -p "$STAGING_DIR/"* "$OUTPUT_DIR/"
            install -m 600 "$STAGING_DIR/.env" "$OUTPUT_DIR/.env"
            echo "已生成安装文件：$OUTPUT_DIR（未构建、未修改系统）"; exit 0
        fi
    fi
    if [[ $ACTION == up || $ACTION == restart ]]; then
        (cd "$REPO_DIR" && cargo build --locked --release -p ostool-server --bins)
        "$REPO_DIR/target/release/ostool-server" --env-file "$ENV_FILE" --check-config
    fi
    SUDO=(); if [[ $(id -u) != 0 ]]; then SUDO=(sudo); fi
    if [[ $EDITION == public ]]; then command -v caddy >/dev/null || { echo '请先安装 Caddy，再重新部署；脚本不修改系统包源。' >&2; exit 1; }; fi
    if [[ $EDITION == local ]] && "${SUDO[@]}" test -f /etc/caddy/ostool-server.caddy; then
        echo '本站点仍有公网 Caddy 配置；请先移除该站点并 reload caddy，再切换免登录 local 版。' >&2
        exit 1
    fi
    getent passwd ostool >/dev/null || "${SUDO[@]}" useradd --system --user-group --home-dir /var/lib/ostool-server --shell /usr/sbin/nologin ostool
    "${SUDO[@]}" install -d -m 750 -o ostool -g ostool /etc/ostool-server /var/lib/ostool-server
    # 不删除旧目录，替换配置前保留备份。
    if "${SUDO[@]}" test -f /etc/ostool-server/.env; then "${SUDO[@]}" cp -p /etc/ostool-server/.env "/etc/ostool-server/.env.backup.$(date +%Y%m%d%H%M%S)"; fi
    if systemctl cat ostool-server >/dev/null 2>&1; then "${SUDO[@]}" systemctl stop ostool-server; fi
    for binary in ostool-server ostool-server-migrate-boards ostool-server-seed-demo; do
        "${SUDO[@]}" install -m 755 "$REPO_DIR/target/release/$binary" "/usr/local/bin/$binary"
    done
    "${SUDO[@]}" install -m 600 -o ostool -g ostool "$STAGING_DIR/.env" /etc/ostool-server/.env
    "${SUDO[@]}" install -m 644 "$STAGING_DIR/ostool-server.service" /etc/systemd/system/ostool-server.service
    "${SUDO[@]}" install -m 644 "$SERVER_DIR/deploy/tftpd.readonly.rules" /etc/ostool-server-tftp.rules
    if getent group dialout >/dev/null; then "${SUDO[@]}" usermod -aG dialout ostool; fi
    "${SUDO[@]}" systemctl daemon-reload
    "${SUDO[@]}" systemctl enable ostool-server
    "${SUDO[@]}" systemctl restart ostool-server
    if [[ $EDITION == public ]]; then
        # 仅管理本站点片段；不更换默认服务、不重启 Caddy、不覆盖其他站点。
        CADDY_MAIN=/etc/caddy/Caddyfile
        CADDY_SITE=/etc/caddy/ostool-server.caddy
        "${SUDO[@]}" test -f "$CADDY_MAIN" || { echo '请先按 Caddy 官方方式初始化 /etc/caddy/Caddyfile。' >&2; exit 1; }
        "${SUDO[@]}" cat "$CADDY_MAIN" > "$STAGING_DIR/Caddyfile.previous"
        cp "$STAGING_DIR/Caddyfile.previous" "$STAGING_DIR/Caddyfile.next"
        if ! grep -Eq '^[[:space:]]*import[[:space:]]+(/etc/caddy/)?ostool-server\.caddy[[:space:]]*$' "$STAGING_DIR/Caddyfile.next"; then
            printf '\nimport ostool-server.caddy\n' >> "$STAGING_DIR/Caddyfile.next"
        fi
        SITE_EXISTED=false
        if "${SUDO[@]}" test -f "$CADDY_SITE"; then
            SITE_EXISTED=true
            "${SUDO[@]}" cat "$CADDY_SITE" > "$STAGING_DIR/site.previous"
        fi
        restore_site() {
            if $SITE_EXISTED; then "${SUDO[@]}" install -m 644 "$STAGING_DIR/site.previous" "$CADDY_SITE";
            else "${SUDO[@]}" unlink "$CADDY_SITE"; fi
        }
        "${SUDO[@]}" install -m 644 "$STAGING_DIR/ostool-server.caddy" "$CADDY_SITE"
        # 候选文件仍放在 /etc/caddy，保持现有相对 import 的解析目录。
        "${SUDO[@]}" install -m 600 "$STAGING_DIR/Caddyfile.next" /etc/caddy/.ostool-Caddyfile.next
        if ! "${SUDO[@]}" caddy validate --config /etc/caddy/.ostool-Caddyfile.next --adapter caddyfile; then
            restore_site
            "${SUDO[@]}" unlink /etc/caddy/.ostool-Caddyfile.next
            echo 'Caddy 验证失败，已恢复本站点片段，现有主配置及运行站点未改动。' >&2; exit 1
        fi
        "${SUDO[@]}" install -m 600 "$STAGING_DIR/Caddyfile.previous" "/etc/caddy/Caddyfile.backup.$(date +%Y%m%d%H%M%S)"
        "${SUDO[@]}" install -m 644 "$STAGING_DIR/Caddyfile.next" "$CADDY_MAIN"
        "${SUDO[@]}" unlink /etc/caddy/.ostool-Caddyfile.next
        if systemctl is-active --quiet caddy; then
            if ! "${SUDO[@]}" systemctl reload caddy; then
                "${SUDO[@]}" install -m 644 "$STAGING_DIR/Caddyfile.previous" "$CADDY_MAIN"
                restore_site
                echo 'Caddy reload 失败，已恢复原配置；未停止现有服务。' >&2; exit 1
            fi
        else "${SUDO[@]}" systemctl enable --now caddy; fi
    fi
}

main() {
    deploy_parse_args "$@"
    deploy_prepare
    if [[ $MODE == docker ]]; then deploy_docker; else deploy_systemd; fi
}

main "$@"
