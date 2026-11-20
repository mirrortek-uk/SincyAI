-- 0011_observability: device-task assignment tracking for crash/offline recovery (WP-15).
-- Requeue-on-disconnect must never touch tasks past the side-effect boundary.

BEGIN;

ALTER TABLE device_tasks
    ADD COLUMN IF NOT EXISTS assigned_device_id UUID;

CREATE INDEX IF NOT EXISTS idx_device_tasks_assigned
    ON device_tasks (agency_id, assigned_device_id, state);

-- Platform-wide ops gauges for /metrics. SECURITY DEFINER so the metrics
-- scrape bypasses tenant RLS (counts only, no row data leaves the function).
CREATE OR REPLACE FUNCTION sincy_ops_gauges() RETURNS JSONB
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public AS $$
DECLARE
    r JSONB;
BEGIN
    SELECT jsonb_build_object(
        'device_tasks', COALESCE((SELECT jsonb_agg(jsonb_build_object('state', state, 'n', n) ORDER BY state)
                                 FROM (SELECT state, COUNT(*)::int8 AS n FROM device_tasks GROUP BY state) t), '[]'::jsonb),
        'devices_online', (SELECT COUNT(*)::int8 FROM devices WHERE state = 'active' AND last_seen > now() - interval '120 seconds'),
        'adapters_degraded', (SELECT COUNT(*)::int8 FROM app_adapter_versions WHERE state = 'degraded'),
        'ui_drift_last_hour', (SELECT COUNT(*)::int8 FROM ui_drift_events WHERE at > now() - interval '1 hour')
    ) INTO r;
    RETURN r;
END $$;
REVOKE EXECUTE ON FUNCTION sincy_ops_gauges() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION sincy_ops_gauges() TO postgres;

COMMIT;
