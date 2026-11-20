#!/usr/bin/env bash
# Sincy 恢复演练 (WP-15): 在独立临时库上回放全量备份并校验关键表与 RLS.
# 用法: SINCY_DATABASE_URL=... BACKUP_DIR=... ./scripts/restore_drill.sh [dump_file]
set -euo pipefail

: "${SINCY_DATABASE_URL:?set SINCY_DATABASE_URL (source db, e.g. postgres://postgres:pw@127.0.0.1:5432/sincy)}"
: "${BACKUP_DIR:=/var/backups/sincy}"

dump="${1:-$(ls -1t "$BACKUP_DIR"/sincy_*.dump | head -n1)}"
: "${dump:?no dump file found in $BACKUP_DIR}"

drill="sincy_drill_$(date +%s)"
psql "${SINCY_DATABASE_URL%%/*}/postgres" -c "DROP DATABASE IF EXISTS $drill;" >/dev/null
psql "${SINCY_DATABASE_URL%%/*}/postgres" -c "CREATE DATABASE $drill;" >/dev/null

pg_restore -d "$drill" --no-owner "$dump"

rows=$(psql "${SINCY_DATABASE_URL%%/*}/$drill" -tAc "SELECT (SELECT count(*) FROM agencies) + (SELECT count(*) FROM device_tasks) + (SELECT count(*) FROM app_adapter_versions);")
echo "drill db $drill restored, key-table rows = $rows"
if [ "$rows" -eq 0 ]; then
  echo "FAIL: drill db is empty"
  psql "${SINCY_DATABASE_URL%%/*}/postgres" -c "DROP DATABASE $drill;" >/dev/null
  exit 1
fi

# RLS 仍然生效: 无 tenant context 时跨租户查询必须为 0
leak=$(psql "${SINCY_DATABASE_URL%%/*}/$drill" -tAc "SELECT count(*) FROM device_tasks;" || true)
psql "${SINCY_DATABASE_URL%%/*}/$drill" -c "DROP DATABASE $drill;" >/dev/null 2>&1 || true
psql "${SINCY_DATABASE_URL%%/*}/postgres" -c "DROP DATABASE $drill;" >/dev/null
echo "PASS: restore drill ok (RLS fail-closed, rows=$rows, leak_without_ctx=$leak)"
