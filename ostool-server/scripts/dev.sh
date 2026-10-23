#!/usr/bin/env bash
set -Eeuo pipefail
# 调试与正式 Docker 部署共用配置和实现；保留熟悉的调试命令。
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
case "${1:-start}" in
    start) action=up;; stop) action=down;; restart|build|status|logs|seed|cert) action=$1;;
    -h|--help|help) echo "用法：$0 {start|stop|restart|build|status|logs|seed|cert} [--env-file 路径]"; exit 0;;
    *) echo "未知命令：$1" >&2; exit 2;;
esac
if (($#)); then shift; fi
exec "$SCRIPT_DIR/deploy.sh" docker "$action" "$@"
