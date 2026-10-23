#!/usr/bin/env bash
set -Eeuo pipefail
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
SERVER_DIR=$(cd -- "$SCRIPT_DIR/.." && pwd)
REPO_DIR=$(cd -- "$SERVER_DIR/.." && pwd)
usage() {
    cat <<'EOF'
用法：deploy.sh {docker|systemd} {up|down|restart|build|status|logs|check|seed|cert} [选项]
  --env-file 路径    唯一配置文件，默认 ostool-server/.env
  --dry-run         仅显示执行计划，不构建、不安装、不重启
  --output-dir 路径  systemd 专用：只生成安装文件，不修改宿主系统
Docker 模式管理 PostgreSQL、Redis 和后端；公网入口接入现有 Caddy。
systemd 模式使用已配置的 PostgreSQL/Redis；up/restart 构建后安装服务。
首次部署或更新使用 up；已有配置和数据库不会被清空。
EOF
}
[[ ${1:-} != -h && ${1:-} != --help && $# -ge 2 ]] || { usage; exit 0; }
MODE=$1 ACTION=$2; shift 2
[[ $MODE == docker || $MODE == systemd ]] || { usage >&2; exit 2; }
case $ACTION in up|down|restart|build|status|logs|check|seed|cert) ;; *) usage >&2; exit 2;; esac
ENV_FILE=${OSTOOL_ENV_FILE:-"$SERVER_DIR/.env"}
DRY_RUN=false OUTPUT_DIR=
while (($#)); do
    case $1 in
        --env-file|--output-dir)
            (($#>=2)) || { echo "$1 缺少参数" >&2; exit 2; }
            if [[ $1 == --env-file ]]; then ENV_FILE=$2; else OUTPUT_DIR=$2; fi
            shift 2;;
        --dry-run) DRY_RUN=true; shift;;
        *) echo "未知参数：$1" >&2; exit 2;;
    esac
done
[[ -f $ENV_FILE ]] || { echo "请先复制 $SERVER_DIR/.env.example 为 .env 并配置连接与部署参数。" >&2; exit 1; }
ENV_FILE=$(cd -- "$(dirname -- "$ENV_FILE")" && pwd)/$(basename -- "$ENV_FILE")
# 部署始终以选定文件为准，避免当前终端的旧变量改变版本或连接目标。
for name in ${!OSTOOL_@}; do unset "$name"; done
export OSTOOL_ENV_FILE=$ENV_FILE
COMPOSE=(docker compose --project-name ostool-server --env-file "$ENV_FILE" -f "$SERVER_DIR/deploy/compose.yaml")
# 使用各部署方式已有的 dotenv 解析器，不执行用户配置中的 Shell 代码。
setting() {
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
if [[ $MODE == docker ]]; then
    command -v docker >/dev/null || { echo '需要 Docker 和 Compose 插件' >&2; exit 1; }
else
    command -v node >/dev/null && node -e 'if(!require("node:util").parseEnv)process.exit(1)' || { echo 'systemd 部署构建需要 Node.js 24 和 pnpm 10.33.0' >&2; exit 1; }
fi
EDITION=$(setting OSTOOL_EDITION local)
[[ $EDITION == local || $EDITION == public ]] || { echo 'OSTOOL_EDITION 只能是 local 或 public' >&2; exit 1; }
if [[ $EDITION == public && $ACTION =~ ^(up|restart|check)$ ]]; then
    [[ $(setting OSTOOL_LISTEN_ADDR 127.0.0.1:2999) =~ ^127\.0\.0\.1:[0-9]+$ ]] || { echo 'public 部署的 API 必须绑定 127.0.0.1，由 Caddy 提供 HTTPS。' >&2; exit 1; }
    [[ $(setting OSTOOL_PUBLIC_BASE_URL) == https://* ]] || { echo 'public 部署必须配置 HTTPS 主站地址。' >&2; exit 1; }
fi
run() {
    if $DRY_RUN; then printf '计划：'; printf ' %q' "$@"; printf '\n'; else "$@"; fi
}
if [[ $MODE == docker ]]; then
    [[ -z $OUTPUT_DIR ]] || { echo '--output-dir 仅适用于 systemd' >&2; exit 2; }
    case $ACTION in
        up|restart)
            run "${COMPOSE[@]}" up -d --build --wait --wait-timeout 120;;
        down) run "${COMPOSE[@]}" down;;
        build) run "${COMPOSE[@]}" build server;;
        check) run "${COMPOSE[@]}" config --quiet;;
        status) run "${COMPOSE[@]}" ps -a;;
        logs) run "${COMPOSE[@]}" logs --tail 100;;
        seed)
            run "${COMPOSE[@]}" up -d --wait postgres redis
            run "${COMPOSE[@]}" stop server
            run "${COMPOSE[@]}" run --build --rm --no-deps --entrypoint ostool-server-seed-demo server --env-file /dev/null
            run "${COMPOSE[@]}" up -d --wait;;
        cert) echo '证书由现有 Caddy 管理；内部 CA 根证书位于其数据目录，请按 Caddy 标准文档导出。';;
    esac
    exit 0
fi
# 仅下列安装动作需要提权；check、dry-run 和输出到临时目录不会修改系统。
if $DRY_RUN; then
    echo "计划：systemd $ACTION（$EDITION），配置 $ENV_FILE；保留现有数据。"
    if [[ $EDITION == public ]]; then echo '计划：仅更新本站点片段，验证整份 Caddyfile 后 reload 默认 caddy.service，保留其他站点。'; fi
    exit 0
fi
if [[ $ACTION == check ]]; then
    [[ -n $(setting OSTOOL_DATABASE_URL) ]] || { echo '缺少 OSTOOL_DATABASE_URL' >&2; exit 1; }
    echo "配置检查通过：systemd / $EDITION（未连接数据库，未安装服务）"; exit 0
fi
if [[ $ACTION =~ ^(status|logs)$ ]]; then
    if [[ $ACTION == status ]]; then systemctl status ostool-server --no-pager; else journalctl -u ostool-server -n 100 --no-pager; fi
    exit 0
fi
if [[ $ACTION == cert || $ACTION == seed ]]; then echo 'systemd 证书见 Caddy 数据目录；演示数据使用独立 seed-demo 可执行文件。' >&2; exit 2; fi
STAGING_DIR=$(mktemp -d)
trap 'rm -rf -- "$STAGING_DIR"' EXIT
if [[ -n $OUTPUT_DIR || $ACTION == up || $ACTION == restart ]]; then
    install -m 600 "$ENV_FILE" "$STAGING_DIR/.env"
    install -m 644 "$SERVER_DIR/deploy/ostool-server.service" "$STAGING_DIR/ostool-server.service"
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
if [[ $ACTION == up || $ACTION == restart || $ACTION == build ]]; then
    (cd "$REPO_DIR" && cargo build --locked --release -p ostool-server --bins)
    [[ $ACTION != build ]] || exit 0
    "$REPO_DIR/target/release/ostool-server" --env-file "$ENV_FILE" --check-config
fi
SUDO=(); if [[ $(id -u) != 0 ]]; then SUDO=(sudo); fi
if [[ $ACTION == down ]]; then
    "${SUDO[@]}" systemctl disable --now ostool-server
    exit 0
fi
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
