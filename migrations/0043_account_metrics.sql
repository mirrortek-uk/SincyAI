-- 0043_account_metrics: 粉丝数测量回流 patch-A (schema + capability)
--
-- account_metric_samples: 平台无关账号指标时间序列 (只存快照, Δ 按窗口现算)。
-- employee_accounts: 员工 → 平台账号 映射 (account_metrics_loop 据此派发 read_followers)。
-- RLS 口径与 cycle_measurements / 员工子表一致: 不单独上 FORCE RLS,
-- 用 agency_id / account_id 显式 scope, 避免后台测量循环因缺租户上下文 fail-closed。
-- channel_capabilities 增加 read_followers 设备路由行 (每平台一个 <platform>.read_followers 适配器)。

BEGIN;

CREATE TABLE IF NOT EXISTS account_metric_samples (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id   UUID NOT NULL REFERENCES agencies (id),
    brand_id    UUID,
    employee_id UUID REFERENCES digital_employees (id),
    platform    TEXT NOT NULL,               -- x / youtube / xiaohongshu / douyin ...
    account_id  TEXT NOT NULL,
    followers   BIGINT,
    following   BIGINT,
    posts_count BIGINT,
    extras      JSONB NOT NULL DEFAULT '{}'::jsonb,  -- raw 原始串(审计) / task_id(设备回流溯源) 等
    source      TEXT NOT NULL DEFAULT 'device' CHECK (source IN ('device','api')),
    observed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_ams_account_time ON account_metric_samples (platform, account_id, observed_at DESC);
CREATE INDEX IF NOT EXISTS idx_ams_employee_time ON account_metric_samples (employee_id, observed_at DESC);

CREATE TABLE IF NOT EXISTS employee_accounts (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id   UUID NOT NULL REFERENCES agencies (id),
    employee_id UUID NOT NULL REFERENCES digital_employees (id) ON DELETE CASCADE,
    platform    TEXT NOT NULL,
    account_id  TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (platform, account_id)
);
CREATE INDEX IF NOT EXISTS idx_emp_accounts_employee ON employee_accounts (employee_id);

-- read_followers 设备路由 capability (X 先行; 每平台各一个 <platform>.read_followers 适配器即扩展)
INSERT INTO channel_capabilities (platform, action, route, status, priority, constraints)
VALUES ('x', 'read_followers', 'device', 'verified', 50, '{"adapter":"x.read_followers","app_package":"com.twitter.android"}')
ON CONFLICT (platform, action, route) DO UPDATE SET constraints = EXCLUDED.constraints;

COMMIT;
