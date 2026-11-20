# Sincy 备份与恢复 (WP-15)

## 1. 目标

- RPO ≤ 5 分钟（WAL 持续归档 + PITR）
- RTO ≤ 30 分钟（见恢复演练脚本）
- 对象存储（S3/本地）生命周期：原始素材 90 天、证据截图（脱敏后）30 天、传输会话残留 7 天后清理

## 2. 数据库（PostgreSQL）

### 2.1 基础配置（启用 WAL 归档）

```ini
# postgresql.conf
wal_level = replica
archive_mode = on
archive_command = 'test ! -f /var/lib/postgresql/wal_archive/%f && cp %p /var/lib/postgresql/wal_archive/%f'
max_wal_senders = 4
```

### 2.2 每日全量备份

`scripts/backup.sh`：

```bash
pg_dump -Fc -d "$SINCY_DATABASE_URL" -f "$BACKUP_DIR/sincy_$(date +%F).dump"
```

保留最近 7 份全量 + 全部 WAL。

### 2.3 PITR 恢复

1. 停止 cloud-api / workers（防止写入旧库）
2. `pg_restore` 全量到新库，或直接在新 data dir 上回放 WAL：
   ```ini
   # recovery.conf / postgresql.auto.conf
   restore_command = 'cp /var/lib/postgresql/wal_archive/%f %p'
   recovery_target_time = '<事故时间点前 1 分钟>'
   ```
3. `scripts/restore_drill.sh` 在独立实例上自动完成：建临时库 → 回放到目标时间 → 校验行数（agencies/device_tasks 不为 0）→ 销毁。

## 3. 对象存储

- 传输会话（`transfer_sessions`）7 天后由 scheduler 清理过期分片
- 证据截图先脱敏（去除账号名/手机号等个人信息）再上传
- 删除一律走生命周期策略，不手动删桶

## 4. 恢复演练（每季度一次）

`scripts/restore_drill.sh` 验收标准：演练库可查询、关键表行数与演练前一致、RLS 策略仍然生效（跨租户查询为 0）。演练记录归档到 `docs/drills/`。
