#!/usr/bin/env bash
set -Eeuo pipefail

# Local development environment for ostool-server. This intentionally uses a
# repository-local data directory and PostgreSQL in both editions; it does
# not modify systemd or /etc/ostool-server.

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO_DIR=$(cd -- "$SCRIPT_DIR/../.." && pwd)
DEV_ENV_FILE=${OSTOOL_DEV_ENV_FILE:-"$SCRIPT_DIR/../.env"}
if [[ "${OSTOOL_DEV_CONFIG_LOADED:-}" != "1" ]]; then
    if [[ -f "$DEV_ENV_FILE" || -v OSTOOL_DEV_ENV_FILE ]]; then
        command -v node >/dev/null 2>&1 || { echo "dev: Node.js is required to read .env" >&2; exit 1; }
        exec node "$SCRIPT_DIR/dev-config.mjs" "$DEV_ENV_FILE" "$SCRIPT_DIR/dev.sh" "$@"
    fi
fi
EDITION=${OSTOOL_EDITION:-local}
[[ "$EDITION" == "local" || "$EDITION" == "public" ]] || { echo "OSTOOL_EDITION must be local or public" >&2; exit 2; }
STATE_DIR=${OSTOOL_DEV_DIR:-"$REPO_DIR/.ostool-server-dev/$EDITION"}
CONFIG_FILE="$STATE_DIR/config.toml"
PID_FILE="$STATE_DIR/server.pid"
LOG_FILE="$STATE_DIR/server.log"
PG_CONTAINER=${OSTOOL_DEV_PG_CONTAINER:-ostool-server-dev-${EDITION}-postgres}
if [[ "$EDITION" == "local" ]]; then DEFAULT_PG_PORT=54328; else DEFAULT_PG_PORT=54329; fi
PG_PORT=${OSTOOL_DEV_PG_PORT:-$DEFAULT_PG_PORT}
REDIS_CONTAINER=${OSTOOL_DEV_REDIS_CONTAINER:-ostool-server-dev-public-redis}
REDIS_PORT=${OSTOOL_DEV_REDIS_PORT:-56380}
SERVER_PORT=${OSTOOL_DEV_SERVER_PORT:-2999}
DEFAULT_SERVER_HOST=0.0.0.0
SERVER_HOST=${OSTOOL_DEV_HOST:-$DEFAULT_SERVER_HOST}
HTTPS_HOST=
if [[ "$EDITION" == "public" ]]; then HTTPS_HOST=${OSTOOL_DEV_HTTPS_HOST:-}; fi
# TLS 入口由 Caddy 绑定局域网，HTTP 调试监听选项不会把代理后端暴露出去。
if [[ -n "$HTTPS_HOST" ]]; then SERVER_HOST=127.0.0.1; fi
LISTEN_HOST=$SERVER_HOST
[[ "$LISTEN_HOST" == *:* ]] && LISTEN_HOST="[$LISTEN_HOST]"
case "$SERVER_HOST" in
    0.0.0.0) CHECK_HOST=127.0.0.1 ;;
    ::) CHECK_HOST='[::1]' ;;
    *) CHECK_HOST=$LISTEN_HOST ;;
esac
CHECK_URL="http://${CHECK_HOST}:${SERVER_PORT}"
HTTPS_PORT=${OSTOOL_DEV_HTTPS_PORT:-3443}
CADDY_CONTAINER=${OSTOOL_DEV_CADDY_CONTAINER:-ostool-server-dev-public-caddy}
PUBLIC_URL=$CHECK_URL
if [[ -n "$HTTPS_HOST" ]]; then
    HTTPS_ADDRESS=$HTTPS_HOST
    [[ "$HTTPS_ADDRESS" == *:* ]] && HTTPS_ADDRESS="[$HTTPS_ADDRESS]"
    PUBLIC_URL="https://${HTTPS_ADDRESS}:${HTTPS_PORT}"
fi
DB_USER=${OSTOOL_DEV_DB_USER:-ostool}
DB_PASSWORD=${OSTOOL_DEV_DB_PASSWORD:-ostool}
DB_NAME=${OSTOOL_DEV_DB_NAME:-ostool}
ADMIN_EMAIL=${OSTOOL_ADMIN_EMAIL:-admin@example.com}
ADMIN_PASSWORD=${OSTOOL_ADMIN_PASSWORD:-admin-password-123}
DEMO=${OSTOOL_DEV_DEMO:-true}
[[ "$DEMO" == "true" || "$DEMO" == "false" ]] || { echo "OSTOOL_DEV_DEMO must be true or false" >&2; exit 2; }

die() { echo "dev: $*" >&2; exit 1; }
log() { echo "dev: $*"; }

require_command() {
    command -v "$1" >/dev/null 2>&1 || die "missing required command: $1";
}

build_server() {
    require_command cargo
    require_command pnpm
    require_command node
    log "building frontend and Rust server (Cargo build.rs embeds both website and admin assets)"
    (
        cd -- "$REPO_DIR"
        # Always build this checkout's UI instead of an externally supplied dist.
        env -u OSTOOL_SERVER_WEB_DIST_DIR cargo build -p ostool-server
    )
    log "build complete; use restart to load the updated executable if the server is running"
}

server_pid() {
    [[ -f "$PID_FILE" ]] || return 1
    local pid
    pid=$(cat "$PID_FILE")
    [[ "$pid" =~ ^[0-9]+$ ]] || return 1
    kill -0 "$pid" 2>/dev/null || return 1
    echo "$pid"
}

write_config() {
    mkdir -p "$STATE_DIR/data"
    mkdir -p "$STATE_DIR/postgres"
    [[ -f "$CONFIG_FILE" ]] && return
    cat > "$CONFIG_FILE" <<EOF
listen_addr = "${LISTEN_HOST}:${SERVER_PORT}"
data_dir = "${STATE_DIR}/data"

[auth]
enabled = true
allow_registration = true
access_token_ttl_secs = 900
refresh_token_ttl_secs = 2592000
device_code_ttl_secs = 600

[tftp]
provider = "builtin"
enabled = true
root_dir = "${STATE_DIR}/data/tftp"
bind_addr = "127.0.0.1:3069"

[network]
interface = "lo"

[network_test]
enabled = false
listen_addr = "127.0.0.1:3000"
max_active_tests = 4
max_duration_secs = 60
EOF
}

ensure_postgres() {
    require_command docker
    local running
    running=$(docker inspect -f '{{.State.Running}}' "$PG_CONTAINER" 2>/dev/null || true)
    if [[ "$running" == "true" ]]; then
        return
    elif docker inspect "$PG_CONTAINER" >/dev/null 2>&1; then
        docker start "$PG_CONTAINER" >/dev/null
    else
        docker run -d \
            --name "$PG_CONTAINER" \
            -e POSTGRES_USER="$DB_USER" \
            -e POSTGRES_PASSWORD="$DB_PASSWORD" \
            -e POSTGRES_DB="$DB_NAME" \
            -p "127.0.0.1:${PG_PORT}:5432" \
            -v "$STATE_DIR/postgres:/var/lib/postgresql/data" \
            postgres:16 >/dev/null
    fi

    for _ in {1..30}; do
        if docker exec "$PG_CONTAINER" pg_isready -U "$DB_USER" -d "$DB_NAME" >/dev/null 2>&1; then
            return
        fi
        sleep 1
    done
    die "PostgreSQL did not become ready; see: docker logs $PG_CONTAINER"
}

ensure_redis() {
    [[ "$EDITION" == "public" ]] || return 0
    mkdir -p "$STATE_DIR/redis"
    if ! docker inspect "$REDIS_CONTAINER" >/dev/null 2>&1; then
        docker run -d --name "$REDIS_CONTAINER" -p "127.0.0.1:${REDIS_PORT}:6379" \
            -v "$STATE_DIR/redis:/data" redis:7-alpine \
            redis-server --appendonly yes --maxmemory 64mb --maxmemory-policy noeviction >/dev/null
    elif [[ $(docker inspect -f '{{.State.Running}}' "$REDIS_CONTAINER") != "true" ]]; then
        docker start "$REDIS_CONTAINER" >/dev/null
    fi
    for _ in {1..30}; do
        if [[ $(docker exec "$REDIS_CONTAINER" redis-cli ping 2>/dev/null) == "PONG" ]]; then return; fi
        sleep 1
    done
    die "Redis did not become ready"
}


start_caddy() {
    local directory="$STATE_DIR/caddy"
    mkdir -p "$directory/data" "$directory/config"
    chmod 700 "$directory/data"
    node "$SCRIPT_DIR/dev-config.mjs" --caddyfile "$HTTPS_HOST" "$HTTPS_PORT" "$SERVER_PORT" > "$directory/Caddyfile"
    if docker inspect "$CADDY_CONTAINER" >/dev/null 2>&1; then
        [[ $(docker inspect -f '{{index .Config.Labels "ostool.dev"}}' "$CADDY_CONTAINER") == "$STATE_DIR" ]] || die "Caddy 容器名称被其他实例使用：$CADDY_CONTAINER"
        docker rm -f "$CADDY_CONTAINER" >/dev/null
    fi
    docker run -d --name "$CADDY_CONTAINER" --network host --label "ostool.dev=$STATE_DIR" \
        -v "$directory/Caddyfile:/etc/caddy/Caddyfile:ro" -v "$directory/data:/data" -v "$directory/config:/config" \
        caddy:2-alpine >/dev/null
    for _ in {1..30}; do
        if docker cp "$CADDY_CONTAINER:/data/caddy/pki/authorities/local/root.crt" "$directory/root.crt" >/dev/null 2>&1 \
            && curl --noproxy '*' --cacert "$directory/root.crt" -fsS "$PUBLIC_URL/api/v1/auth/config" >/dev/null 2>&1; then
            log "HTTPS CA certificate: $directory/root.crt (在访问设备上导入并信任，重启会保留此证书)"
            return
        fi
        sleep 1
    done
    die "Caddy HTTPS 未就绪，请查看 docker logs $CADDY_CONTAINER"
}

start_server() {
    require_command cargo
    require_command pnpm
    require_command node
    node -e 'if (!require("node:net").isIP(process.argv[1])) process.exit(1)' "$SERVER_HOST" \
        || die "OSTOOL_DEV_HOST must be an IPv4 or IPv6 address (without brackets)"
    node "$SCRIPT_DIR/dev-config.mjs" --check-network "$SERVER_HOST" "$SERVER_PORT" "$HTTPS_HOST" "$HTTPS_PORT" || exit 1
    if [[ -n "$HTTPS_HOST" ]]; then
        [[ "$EDITION" == "public" ]] || die "HTTPS 调试入口仅用于 public 版"
        [[ $(uname -s) == Linux ]] || die "自动 Caddy 调试代理需要 Linux Docker host 网络"
        require_command curl
    fi
    mkdir -p "$STATE_DIR"
    write_config
    local pid
    if pid=$(server_pid); then
        log "ostool-server is already running (pid $pid)"
        log "use restart to rebuild embedded web assets and Rust, then load the updated executable"
        return
    fi
    if [[ -n "$HTTPS_HOST" ]]; then
        node - "$HTTPS_HOST" "$HTTPS_PORT" <<'JS' || die "HTTPS 调试地址或端口不可用，请检查 OSTOOL_DEV_HTTPS_HOST/OSTOOL_DEV_HTTPS_PORT"
const server = require("node:net").createServer();
server.on("error", () => { process.exitCode = 1; });
server.listen({host:process.argv[2],port:Number(process.argv[3])},()=>server.close());
JS
    fi
    # Do not mistake another edition's HTTP response for this process being ready.
    # Probe before starting databases or spawning a build in the background.
    if ! node - "$SERVER_HOST" "$SERVER_PORT" <<'JS'
const server = require("node:net").createServer();
server.on("error", () => { process.exitCode = 1; });
server.listen({host: process.argv[2], port: Number(process.argv[3])}, () => server.close());
JS
    then
        die "cannot bind ${LISTEN_HOST}:${SERVER_PORT}; another edition/process may be using it. Stop that edition or change OSTOOL_DEV_SERVER_PORT"
    fi
    rm -f "$PID_FILE"
    ensure_postgres
    ensure_redis
    log "仅启动调试环境（local/public 功能可选）；正式部署请使用 scripts/install.sh 与 Caddy"
    log "starting ostool-server; log: $LOG_FILE"
    # Only allowlisted development settings are loaded above. The server itself
    # receives an empty env file and explicit development database/service URLs.
    : > "$STATE_DIR/server.env"
    if [[ "$EDITION" == "public" && ! -f "$STATE_DIR/mfa.key" ]]; then
        require_command node
        (umask 077; node -e 'process.stdout.write(require("node:crypto").randomBytes(32).toString("base64"))' > "$STATE_DIR/mfa.key")
    fi
    local -a server_env=(env -u OSTOOL_DATABASE_URL -u OSTOOL_ADMIN_EMAIL -u OSTOOL_ADMIN_PASSWORD -u OSTOOL_SERVER_WEB_DIST_DIR
        "OSTOOL_EDITION=$EDITION" "OSTOOL_LISTEN_ADDR=${LISTEN_HOST}:${SERVER_PORT}"
        "OSTOOL_DATABASE_URL=postgres://${DB_USER}:${DB_PASSWORD}@127.0.0.1:${PG_PORT}/${DB_NAME}"
        "OSTOOL_PUBLIC_BASE_URL=$PUBLIC_URL" "OSTOOL_NETWORK_TEST_ENABLED=false")
    if [[ -n "$HTTPS_HOST" ]]; then server_env+=("OSTOOL_TRUSTED_PROXY_IPS=127.0.0.1,::1"); fi
    if [[ "$EDITION" == "public" ]]; then
        server_env+=("OSTOOL_DATABASE_URL=postgres://${DB_USER}:${DB_PASSWORD}@127.0.0.1:${PG_PORT}/${DB_NAME}"
            "OSTOOL_ADMIN_EMAIL=$ADMIN_EMAIL" "OSTOOL_ADMIN_PASSWORD=$ADMIN_PASSWORD" "OSTOOL_AUTH_ENABLED=true"
            "OSTOOL_REDIS_URL=redis://127.0.0.1:${REDIS_PORT}/0"
            "OSTOOL_MFA_ENCRYPTION_KEY=$(cat "$STATE_DIR/mfa.key")")
    fi
    local -a server_args=(--dev)
    if [[ "$DEMO" == "true" ]]; then
        server_args+=(--dev-demo)
        log "demo fixtures enabled (demo-*); hardware discovery and startup power-off disabled"
    fi
    # A separate session survives callers that terminate their process group
    # after this launcher exits (IDE terminals and command runners). Keep the
    # foreground child PID so stop/status still address the actual server.
    local -a detach=()
    if command -v setsid >/dev/null 2>&1; then detach=(setsid); fi
    nohup "${detach[@]}" "${server_env[@]}" \
        bash -c 'cd "$1" && shift && exec cargo run -p ostool-server -- --config "$1" --env-file "$2" "${@:3}"' \
        dev "$REPO_DIR" "$CONFIG_FILE" "$STATE_DIR/server.env" "${server_args[@]}" \
        >"$LOG_FILE" 2>&1 < /dev/null &
    echo $! > "$PID_FILE"
    for _ in {1..30}; do
        if ! server_pid >/dev/null; then
            break
        fi
        if command -v curl >/dev/null 2>&1 && curl --noproxy '*' -fsS "$CHECK_URL/" >/dev/null 2>&1; then
            if [[ -n "$HTTPS_HOST" ]] && ! (start_caddy); then
                stop_server
                die "HTTPS 调试入口启动失败，已关闭本次开发环境；请检查 Caddy 配置后重试"
            fi
            log "ready: $PUBLIC_URL/"
            log "admin: $PUBLIC_URL/admin/overview"
            log "listening: ${LISTEN_HOST}:${SERVER_PORT}"
            if [[ "$SERVER_HOST" == "0.0.0.0" ]]; then
                local lan_ip
                while IFS= read -r lan_ip; do
                    log "LAN: http://${lan_ip}:${SERVER_PORT}/"
                done < <(node -e 'for (const [name, addresses] of Object.entries(require("node:os").networkInterfaces())) {
                    if (/^(docker|veth|br-|virbr)/.test(name)) continue;
                    for (const address of addresses ?? []) if (address.family === "IPv4" && !address.internal) console.log(address.address);
                }')
            fi
            if [[ "$EDITION" == "public" ]]; then
                log "website login: $PUBLIC_URL/login"
                log "admin login: $PUBLIC_URL/admin/login"
            fi
            return
        fi
        sleep 1
    done
    if ! server_pid >/dev/null; then
        tail -n 40 "$LOG_FILE" >&2 || true
        die "ostool-server failed to start"
    fi
    log "process started; readiness check is unavailable or still pending"
}

stop_server() {
    if [[ "$EDITION" == "public" ]] && command -v docker >/dev/null 2>&1; then
        if [[ $(docker inspect -f '{{index .Config.Labels "ostool.dev"}}' "$CADDY_CONTAINER" 2>/dev/null || true) == "$STATE_DIR" ]]; then
            docker stop -t 3 "$CADDY_CONTAINER" >/dev/null 2>&1 || true
        fi
    fi
    local pid
    if pid=$(server_pid); then
        log "stopping ostool-server (pid $pid)"
        kill "$pid" 2>/dev/null || true
        for _ in {1..20}; do
            kill -0 "$pid" 2>/dev/null || break
            sleep 0.25
        done
        if kill -0 "$pid" 2>/dev/null; then
            kill -KILL "$pid" 2>/dev/null || true
        fi
    else
        log "ostool-server is not running"
    fi
    rm -f "$PID_FILE"
    if [[ "$EDITION" == "public" ]] && command -v docker >/dev/null 2>&1; then
        docker stop "$REDIS_CONTAINER" >/dev/null 2>&1 || true
    fi
    if command -v docker >/dev/null 2>&1 && docker inspect "$PG_CONTAINER" >/dev/null 2>&1; then
        local running
        running=$(docker inspect -f '{{.State.Running}}' "$PG_CONTAINER" 2>/dev/null || true)
        [[ "$running" == "true" ]] && docker stop "$PG_CONTAINER" >/dev/null
        log "PostgreSQL container stopped: $PG_CONTAINER"
    fi
}

status() {
    local pid
    if pid=$(server_pid); then
        echo "ostool-server: running (pid $pid)"
    else
        echo "ostool-server: stopped"
    fi
    echo "edition: $EDITION"
    echo "requested listener: ${LISTEN_HOST}:${SERVER_PORT} (applied on start/restart)"
    if [[ "$EDITION" == "public" ]]; then
        echo "redis: $(docker inspect -f '{{.State.Running}}' "$REDIS_CONTAINER" 2>/dev/null || echo 'not created') ($REDIS_CONTAINER, port $REDIS_PORT)"
    fi
    if command -v docker >/dev/null 2>&1; then
        local running
        running=$(docker inspect -f '{{.State.Running}}' "$PG_CONTAINER" 2>/dev/null || true)
        echo "postgres: ${running:-not created} ($PG_CONTAINER, port $PG_PORT)"
    else
        echo "postgres: Docker unavailable"
    fi
    if [[ -n "$HTTPS_HOST" ]]; then
        echo "HTTPS: $PUBLIC_URL"
        echo "CA: $STATE_DIR/caddy/root.crt"
    fi
    echo "state: $STATE_DIR"
}

case "${1:-start}" in
    build) build_server ;;
    start) start_server ;;
    stop) stop_server ;;
    restart) stop_server; start_server ;;
    status) status ;;
    clean)
        stop_server
        if [[ -d "$STATE_DIR/postgres" ]] && command -v docker >/dev/null 2>&1; then
            # PostgreSQL writes the bind-mounted data as uid 999. Use the
            # container's root user to remove it without requiring sudo.
            docker run --rm -v "$STATE_DIR/postgres:/data" postgres:16 \
                bash -c 'rm -rf /data/* /data/.[!.]* /data/..?*' >/dev/null
        fi
        if [[ "$EDITION" == "public" ]] && command -v docker >/dev/null 2>&1; then
            if [[ $(docker inspect -f '{{index .Config.Labels "ostool.dev"}}' "$CADDY_CONTAINER" 2>/dev/null || true) == "$STATE_DIR" ]]; then
                docker rm "$CADDY_CONTAINER" >/dev/null 2>&1 || true
            fi
            if [[ -d "$STATE_DIR/caddy/data" ]]; then
                docker run --rm -v "$STATE_DIR/caddy:/data" --entrypoint sh caddy:2-alpine -c 'rm -rf /data/* /data/.[!.]* /data/..?*' >/dev/null
            fi
            docker rm "$REDIS_CONTAINER" >/dev/null 2>&1 || true
            if [[ -d "$STATE_DIR/redis" ]]; then
                docker run --rm -v "$STATE_DIR/redis:/data" --entrypoint sh redis:7-alpine \
                    -c 'rm -rf /data/* /data/.[!.]* /data/..?*' >/dev/null
            fi
        fi
        rm -rf "$STATE_DIR"
        log "removed $STATE_DIR"
        ;;
    help|-h|--help)
        echo "仅用于本机/局域网调试；正式部署使用 scripts/install.sh 与 Caddy"
        echo "usage: $0 {build|start|stop|restart|status|clean}"
        echo "  build    build embedded website/admin assets and Rust; keep running services unchanged"
        echo "  restart  rebuild and restart the selected development edition"
        ;;
    *)
        echo "usage: $0 {build|start|stop|restart|status|clean}" >&2
        exit 2
        ;;
esac
