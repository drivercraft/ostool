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
Docker 模式管理 PostgreSQL、Redis 和服务容器；public 版额外启动 Caddy。
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
    if [[ $EDITION == public ]]; then COMPOSE+=(--profile public); fi
    case $ACTION in
        up|restart)
            if [[ $EDITION == local ]]; then run "${COMPOSE[@]}" stop caddy; fi
            run "${COMPOSE[@]}" up -d --build --remove-orphans --wait --wait-timeout 120;;
        down) run "${COMPOSE[@]}" --profile public down;;
        build) run "${COMPOSE[@]}" build server;;
        check) run "${COMPOSE[@]}" config --quiet;;
        status) run "${COMPOSE[@]}" --profile public ps -a;;
        logs) run "${COMPOSE[@]}" --profile public logs --tail 100;;
        seed)
            run "${COMPOSE[@]}" up -d --wait postgres redis
            run "${COMPOSE[@]}" stop caddy server
            run "${COMPOSE[@]}" run --build --rm --no-deps --entrypoint ostool-server-seed-demo server --env-file /dev/null
            run "${COMPOSE[@]}" up -d --wait;;
        cert)
            [[ $EDITION == public ]] || { echo 'local 版不启用 Caddy，无需导出证书。'; exit 0; }
            run mkdir -p "$REPO_DIR/.ostool-server-dev"
            run "${COMPOSE[@]}" cp caddy:/data/caddy/pki/authorities/local/root.crt "$REPO_DIR/.ostool-server-dev/root.crt";;
    esac
    exit 0
fi
# 仅下列安装动作需要提权；check、dry-run 和输出到临时目录不会修改系统。
if $DRY_RUN; then
    echo "计划：systemd $ACTION（$EDITION），配置 $ENV_FILE；保留现有数据。"
    if [[ $EDITION == public ]]; then echo '计划：额外安装 ostool-server-caddy.service 与 Caddyfile。'; fi
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
        install -m 644 "$SERVER_DIR/deploy/Caddyfile" "$STAGING_DIR/Caddyfile"
        install -m 644 "$SERVER_DIR/deploy/ostool-server-caddy.service" "$STAGING_DIR/ostool-server-caddy.service"
        # 代理只接收其自身参数，不获得数据库、SMTP 或 MFA 密钥。
        for key in OSTOOL_SITE_ADDRESSES OSTOOL_HTTPS_PORT OSTOOL_HTTPS_BIND; do
            value=$(setting "$key")
            value=${value//\\/\\\\}
            if [[ -n $value ]]; then printf '%s="%s"\n' "$key" "${value//\"/\\\"}"; fi
        done > "$STAGING_DIR/caddy.env"
        printf 'OSTOOL_UPSTREAM="%s"\n' "$(setting OSTOOL_LISTEN_ADDR 127.0.0.1:2999)" >> "$STAGING_DIR/caddy.env"
        chmod 600 "$STAGING_DIR/caddy.env"
        # 将非敏感代理参数渲染到 unit，不再生成第二份运行环境文件。
        node --input-type=commonjs - "$STAGING_DIR" <<'JS'
const fs=require('node:fs'),path=require('node:path'),{parseEnv}=require('node:util');
const directory=process.argv[2];
const file=path.join(directory,'ostool-server-caddy.service');
const environment=parseEnv(fs.readFileSync(path.join(directory,'caddy.env'),'utf8'));
const directives=Object.entries(environment).map(([key,value])=>'Environment='+JSON.stringify(key+'='+value)).join('\n');
fs.writeFileSync(file,fs.readFileSync(file,'utf8').replace('[Service]','[Service]\n'+directives));
fs.unlinkSync(path.join(directory,'caddy.env'));
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
    if systemctl cat ostool-server-caddy >/dev/null 2>&1; then "${SUDO[@]}" systemctl disable --now ostool-server-caddy; fi
    exit 0
fi
if [[ $EDITION == public ]]; then command -v caddy >/dev/null || { echo '请先安装 Caddy，再重新部署；脚本不修改系统包源。' >&2; exit 1; }; fi
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
if [[ $EDITION == public ]]; then
    getent passwd ostool-caddy >/dev/null || "${SUDO[@]}" useradd --system --user-group --home-dir /var/lib/ostool-server-proxy --shell /usr/sbin/nologin ostool-caddy
    "${SUDO[@]}" install -d -m 750 -o ostool-caddy -g ostool-caddy /etc/ostool-server-proxy /var/lib/ostool-server-proxy
    "${SUDO[@]}" install -m 644 -o ostool-caddy -g ostool-caddy "$STAGING_DIR/Caddyfile" /etc/ostool-server-proxy/Caddyfile
    "${SUDO[@]}" install -m 644 "$STAGING_DIR/ostool-server-caddy.service" /etc/systemd/system/ostool-server-caddy.service
else
    if systemctl cat ostool-server-caddy >/dev/null 2>&1; then "${SUDO[@]}" systemctl disable --now ostool-server-caddy; fi
fi
"${SUDO[@]}" systemctl daemon-reload
"${SUDO[@]}" systemctl enable ostool-server
"${SUDO[@]}" systemctl restart ostool-server
if [[ $EDITION == public ]]; then "${SUDO[@]}" systemctl enable ostool-server-caddy; "${SUDO[@]}" systemctl restart ostool-server-caddy; fi
