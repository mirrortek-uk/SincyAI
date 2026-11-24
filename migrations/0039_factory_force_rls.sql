-- 0039: factory 表改为 FORCE ROW LEVEL SECURITY (与 0002/0003/0004 所有租户表一致)
-- ENABLE 不够: 表 owner 会绕过 RLS; FORCE 让 owner 也受 RLS 约束
ALTER TABLE factory_briefs FORCE ROW LEVEL SECURITY;
ALTER TABLE factory_learnings FORCE ROW LEVEL SECURITY;
