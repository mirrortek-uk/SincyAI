-- 0028: 平台审核规则可配置化 (不再硬编码 280)
-- 每个平台的审核门(加权字数上限 + 超限行为)存表, 控制台设置页可改。
-- on_over_limit: 'reject'=超限打回(旧行为) / 'thread'=拆成串行帖发布(X 等支持 thread)
BEGIN;

CREATE TABLE IF NOT EXISTS platform_review_rules (
  platform       TEXT PRIMARY KEY,
  weighted_limit INTEGER NOT NULL CHECK (weighted_limit > 0),
  on_over_limit  TEXT NOT NULL DEFAULT 'reject' CHECK (on_over_limit IN ('reject','thread')),
  note           TEXT NOT NULL DEFAULT '',
  updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 默认值: X 280 加权(中文算 2) 且超限走 thread; 其它平台 40000 拒绝
INSERT INTO platform_review_rules (platform, weighted_limit, on_over_limit, note) VALUES
  ('x',       280,   'thread', 'X 帖子上限 280 加权字符(CJK 算 2); 超限自动拆成串行帖'),
  ('default', 40000, 'reject', '未单独配置的平台使用的默认审核门')
ON CONFLICT (platform) DO NOTHING;

-- 发布包支持 thread: 拆分后的分块(为空表示单帖)
ALTER TABLE publishing_packages ADD COLUMN IF NOT EXISTS thread_chunks JSONB NOT NULL DEFAULT '[]';

COMMIT;
