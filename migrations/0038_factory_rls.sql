-- 0038: factory_briefs / factory_learnings 租户隔离
-- 1) 回填孤儿行到首个 agency, 收紧 agency_id 为 NOT NULL + FK
UPDATE factory_briefs
   SET agency_id = (SELECT id FROM agencies ORDER BY created_at LIMIT 1)
 WHERE agency_id IS NULL;
ALTER TABLE factory_learnings ADD COLUMN IF NOT EXISTS agency_id UUID;
UPDATE factory_learnings
   SET agency_id = (SELECT id FROM agencies ORDER BY created_at LIMIT 1)
 WHERE agency_id IS NULL;

ALTER TABLE factory_briefs
    ALTER COLUMN agency_id SET NOT NULL,
    ADD CONSTRAINT factory_briefs_agency_fk FOREIGN KEY (agency_id) REFERENCES agencies (id);
ALTER TABLE factory_learnings
    ALTER COLUMN agency_id SET NOT NULL,
    ADD CONSTRAINT factory_learnings_agency_fk FOREIGN KEY (agency_id) REFERENCES agencies (id);

-- 2) RLS: 与其它租户表一致 (app.current_agency_id 由请求事务 SET LOCAL 注入)
ALTER TABLE factory_briefs ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON factory_briefs
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

ALTER TABLE factory_learnings ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON factory_learnings
    USING (agency_id::text = current_setting('app.current_agency_id', true))
    WITH CHECK (agency_id::text = current_setting('app.current_agency_id', true));

-- 3) service role (sincy_app) 绕过 RLS 用于后台 runner
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_policies WHERE tablename = 'factory_briefs' AND policyname = 'service_all') THEN
        CREATE POLICY service_all ON factory_briefs FOR ALL TO sincy_app USING (true) WITH CHECK (true);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_policies WHERE tablename = 'factory_learnings' AND policyname = 'service_all') THEN
        CREATE POLICY service_all ON factory_learnings FOR ALL TO sincy_app USING (true) WITH CHECK (true);
    END IF;
END $$;
