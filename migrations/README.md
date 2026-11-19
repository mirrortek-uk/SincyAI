# migrations/

有编号的 SQL 迁移（sqlx migrate）。规则（指导书 6）：
- 先导出现有 schema，确认父表与字段类型，再写 migration；
- 引用 (id, agency_id) 前，父表必须存在 UNIQUE(id, agency_id)；
- 平台级表（platform_apps、quota_pools、channel_capabilities、app_adapters）
  不启用租户 RLS，写权限限 Platform Admin；
- task_events / execution_steps / audit_events / outbox_events 只追加写；
- 迁移必须在空库和升级库都可运行（DoD）。
