-- 0018_intel_jobs: unique key for collection job upserts + seed job rows for
-- the collector (WI §9). The collector itself is a cloud-api background task
-- (intel_collector.rs) that only touches registered read-only public sources.

BEGIN;

ALTER TABLE intel_collection_jobs
    ADD CONSTRAINT intel_collection_jobs_source_schedule_key UNIQUE (source_id, schedule);

INSERT INTO intel_collection_jobs (source_id, schedule, status) VALUES
  ('rsshub', 'feed:ithome', 'idle'),
  ('rsshub', 'feed:dev.to #ai', 'idle'),
  ('rsshub', 'feed:sspai', 'idle'),
  ('hotrank-hub', 'feed:github trending repos (ai agent)', 'idle'),
  ('hotrank-hub', 'feed:github trending repos (llm)', 'idle'),
  ('searxng', 'feed:searxng: AI agents 2026', 'idle'),
  ('searxng', 'feed:searxng: 多智能体 协作', 'idle'),
  ('trendradar', 'hourly', 'idle'),
  ('reach-mcp', 'hourly', 'idle')
ON CONFLICT (source_id, schedule) DO NOTHING;

COMMIT;
