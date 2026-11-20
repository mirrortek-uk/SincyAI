#!/usr/bin/env bash
# Sincy 日全量备份 (WP-15). 需设置 SINCY_DATABASE_URL 与 BACKUP_DIR.
# 保留最近 7 份全量; WAL 归档由 postgresql.conf 的 archive_command 持续写入.
set -euo pipefail

: "${SINCY_DATABASE_URL:?set SINCY_DATABASE_URL}"
: "${BACKUP_DIR:=/var/backups/sincy}"

mkdir -p "$BACKUP_DIR"
stamp="$(date +%F)"
out="$BACKUP_DIR/sincy_${stamp}.dump"

pg_dump -Fc -d "$SINCY_DATABASE_URL" -f "$out"
echo "wrote $out"

# 保留最近 7 份
ls -1t "$BACKUP_DIR"/sincy_*.dump | tail -n +8 | xargs -r rm -f
echo "retention: last 7 full dumps kept in $BACKUP_DIR"
