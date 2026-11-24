-- 0048: 机会 per-employee 认领 + 动态池上限
--
-- 设计动机: intel_opportunities 是全平台共享情报池, 不能被单个员工独占消费
-- (否则 2k 员工会把 top-500 的 new 池抢光)。改为每个员工独立认领:
--   同一 topic 可被不同租户/员工分别消费, 各自基于自己的 voice/focus/pillars 生成不同内容;
--   同一员工不会重复认领同一 topic (UNIQUE(employee_id, topic)).
--
-- 与 cycle_measurements/employee_accounts 一致: 不单独上 FORCE RLS,
-- 用 agency_id 列显式 scope, 避免后台循环缺租户上下文时 fail-closed。

CREATE TABLE IF NOT EXISTS opportunity_claims (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id   UUID NOT NULL,
    employee_id UUID NOT NULL REFERENCES digital_employees(id) ON DELETE CASCADE,
    topic       TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'selected',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (employee_id, topic)
);
CREATE INDEX IF NOT EXISTS idx_opp_claims_agency ON opportunity_claims (agency_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_opp_claims_employee ON opportunity_claims (employee_id, created_at DESC);

-- 后台 expire 任务在无租户上下文下跑 (RLS 对 sincy_app 生效), 用 SECURITY DEFINER
-- 绕过 digital_employees 的 tenant_isolation policy, 统计全平台活跃员工数,
-- 供机会池上限动态化: cap = GREATEST(500, 50 * 活跃员工数)。
CREATE OR REPLACE FUNCTION platform_active_employee_count() RETURNS BIGINT
LANGUAGE sql SECURITY DEFINER STABLE AS $$
    SELECT COUNT(*) FROM digital_employees WHERE status IN ('RUNNING', 'PAUSED')
$$;
