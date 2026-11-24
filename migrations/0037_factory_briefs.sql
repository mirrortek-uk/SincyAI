-- 0037: 内容工厂持久化 — Brief (工序0) 与 Learn 结论 (工序9 回写)
CREATE TABLE IF NOT EXISTS factory_briefs (
    brief_id    TEXT PRIMARY KEY,
    agency_id   UUID,
    brand_id    TEXT,
    trigger     TEXT NOT NULL,              -- opportunity_driven / calendar_driven / manual
    seed_opportunity_id TEXT,
    seed_signal_id      TEXT,
    goal        TEXT NOT NULL DEFAULT '',
    platforms   JSONB NOT NULL DEFAULT '[]',
    angle       TEXT NOT NULL DEFAULT '',
    audience    TEXT NOT NULL DEFAULT '',
    cta         TEXT NOT NULL DEFAULT '',
    constraints JSONB NOT NULL DEFAULT '{}',
    status      TEXT NOT NULL DEFAULT 'draft',  -- draft / running / done / failed
    result      JSONB,                       -- plan/master/variants 产出
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_factory_briefs_brand ON factory_briefs (brand_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_factory_briefs_trigger ON factory_briefs (trigger, status);

CREATE TABLE IF NOT EXISTS factory_learnings (
    id          BIGSERIAL PRIMARY KEY,
    brand_id    TEXT,
    employee_id UUID REFERENCES digital_employees (id),
    hypothesis  TEXT NOT NULL,
    evidence    JSONB NOT NULL DEFAULT '[]',
    confidence  DOUBLE PRECISION NOT NULL DEFAULT 0,
    source      TEXT NOT NULL DEFAULT 'factory_learn',  -- factory_learn / employee_experiments 镜像
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_factory_learnings_brand ON factory_learnings (brand_id, created_at DESC);
