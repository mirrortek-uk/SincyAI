-- 0042_learn_loop: 闭合数字员工学习回路 (a) + 延迟测量 (b)
--
-- (a) employee_experiments 加 adjustment JSONB + applied_at:
--     Adapt 步除假说外写一条结构化调整 {"angle","pillar_boost"},
--     下周期开头 take_pending_adjustment() 原子取用 (SELECT → 标 applied → 返回)。
-- (b) cycle_measurements: 发布后登记, 延迟到期后由后台 drain 读 device_tasks 真实终态,
--     写回 metrics + insight 并产出一条数据驱动 adjustment → 下周期消费。
--
-- 设计取舍: cycle_measurements 不单独上 FORCE RLS (与 operating_cycles /
-- employee_experiments 等员工子表一致), 用 employee_id / agency_id 列显式 scope,
-- 避免后台 drain 因缺租户上下文 fail-closed。

BEGIN;

ALTER TABLE employee_experiments
    ADD COLUMN IF NOT EXISTS adjustment JSONB,
    ADD COLUMN IF NOT EXISTS applied_at TIMESTAMPTZ;

CREATE TABLE IF NOT EXISTS cycle_measurements (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    cycle_id    UUID NOT NULL REFERENCES operating_cycles (id),
    employee_id UUID NOT NULL REFERENCES digital_employees (id),
    agency_id   UUID,                     -- 显式租户 scope (不走 RLS, 后台 drain 无租户上下文)
    package_id  UUID,
    task_id     UUID,                     -- device_tasks.id, drain据此读真实终态
    due_at      TIMESTAMPTZ NOT NULL,
    status      TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','done','skipped')),
    metrics     JSONB,
    insight     TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    measured_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_cycle_measurements_due
    ON cycle_measurements (due_at) WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS idx_cycle_measurements_employee ON cycle_measurements (employee_id);

COMMIT;
