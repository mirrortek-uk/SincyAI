-- 0025: device_tasks 携带任意任务上下文 (如 agentic 模式的 goal), 随任务下发、审计可查
ALTER TABLE device_tasks ADD COLUMN IF NOT EXISTS context JSONB NOT NULL DEFAULT '{}'::jsonb;
