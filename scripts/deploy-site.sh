#!/usr/bin/env bash
# 把生成的下载站（dist/site）同步到网站服务器。
#
# 用法：
#   SITE_SSH_HOST=xyq.mhwy.work \
#   SITE_SSH_USER=deploy \
#   SITE_PATH=/var/www/xyq.mhwy.work \
#   [SITE_SSH_PORT=22] [SITE_SSH_KEY=~/.ssh/deploy_key] \
#   bash scripts/deploy-site.sh [dist/site]
#
# 说明：使用 rsync --delete 覆盖目标目录，SITE_PATH 请使用独立目录。
set -euo pipefail
cd "$(dirname "$0")/.."

SITE_DIR="${1:-dist/site}"
: "${SITE_SSH_HOST:?请设置 SITE_SSH_HOST（服务器地址）}"
: "${SITE_SSH_USER:?请设置 SITE_SSH_USER（SSH 用户）}"
: "${SITE_PATH:?请设置 SITE_PATH（网站目录）}"
PORT="${SITE_SSH_PORT:-22}"

if [ ! -d "$SITE_DIR" ]; then
    echo "站点目录不存在：$SITE_DIR，先运行 python scripts/build-site.py" >&2
    exit 1
fi

SSH_CMD="ssh -p ${PORT} -o StrictHostKeyChecking=accept-new"
if [ -n "${SITE_SSH_KEY:-}" ]; then
    SSH_CMD="${SSH_CMD} -i ${SITE_SSH_KEY} -o IdentitiesOnly=yes"
fi

echo "==> 同步 ${SITE_DIR}/ 到 ${SITE_SSH_HOST}:${SITE_PATH}/"
rsync -az --delete -e "${SSH_CMD}" "${SITE_DIR}/" "${SITE_SSH_USER}@${SITE_SSH_HOST}:${SITE_PATH}/"
echo "==> 完成：https://${SITE_SSH_HOST}/latest.json"
