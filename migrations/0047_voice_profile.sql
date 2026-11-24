-- 0047: Voice Profile(语气档案) 存储落点
--
-- 两级生效 (见 设计书 Sincy-Voice-Profile.md):
--   基线 base    = employee_identity_versions.identity->'voice_profile' (identity 本就是 JSONB, 无需改表)
--   覆盖 override = employee_accounts.voice_profile (本列)
-- 有效语气 = merge_opt(base, override): 覆盖里显式 null 的键保留基线值。
--
-- 注: 代码实际使用的账号登记表是 employee_accounts (0043,供各循环/KPI/UI 使用),
--     social_accounts 虽在 0007 建了但云端代码未接线, 故账号级覆盖落在本表。
-- employee_accounts 无单独 RLS(与 0043 一致), 由 agency_id 列显式 scope。

ALTER TABLE employee_accounts
    ADD COLUMN IF NOT EXISTS voice_profile JSONB;
