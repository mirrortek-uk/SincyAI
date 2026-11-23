-- 0029_fix_youtube_quota_cost: correct the seeded YouTube videos.insert quota
-- cost. Migrations 0003 and 0012 seeded quota_cost=1600 (pre-2026-12-04 doc),
-- which made the shared quota pool exhaust ~16x too fast. The capability
-- matrix is the single source of truth the cloud-api reads at publish time,
-- so correcting it here fixes accounting without a code release. Verify the
-- live cost in the Google Cloud Console before launch.

UPDATE channel_capabilities
   SET constraints = jsonb_set(constraints, '{quota_cost}', '100'::jsonb, true)
 WHERE platform = 'youtube'
   AND action = 'publish_video'
   AND route = 'api'
   AND (constraints->>'quota_cost') = '1600';
