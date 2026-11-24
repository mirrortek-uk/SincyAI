-- 0044: 粉丝运营闭环 (patch-B + 互动子循环)
-- 1) 签名并启用 x.read_followers 只读测量适配器 (dev 签名密钥; prod 由 ops 重新播种, 见 0015 说明)
INSERT INTO app_adapters (id, app_package, description)
VALUES ('x.read_followers', 'com.twitter.android', 'X follower/following read-only measurement via device automation (pure perception; challenge => WAITING_USER)')
ON CONFLICT (id) DO NOTHING;

INSERT INTO app_adapter_versions (adapter_id, version, version_range, definition, signature, sha256, state)
VALUES (
    'x.read_followers',
    1,
    '>=10.0',
    '{"adapter":"x.read_followers","version":1,"app":{"package":"com.twitter.android","version_range":">=10.0"},"screens":{"challenge":{"anchors":[{"id":null,"text":null,"text_any":["人机验证","Verify you are human"],"near_text_any":null}]},"main_nav":{"anchors":[{"id":null,"text":null,"text_any":["主页","Home"],"near_text_any":null},{"id":null,"text":null,"text_any":["通知标签","Notifications"],"near_text_any":null}]},"profile":{"anchors":[{"id":null,"text":null,"text_any":["正在关注","Following"],"near_text_any":null},{"id":null,"text":null,"text_any":["关注者","Followers"],"near_text_any":null}]}},"flow":[{"expect":"main_nav","action":"open_self_profile","target":null,"value":null,"side_effect":false,"capture":null,"verify":false},{"expect":"profile","action":"wait","target":null,"value":"2000","side_effect":false,"capture":null,"verify":false},{"expect":"profile","action":"read","target":{"id":null,"text":null,"text_any":null,"near_text_any":["关注者","Followers"]},"value":null,"side_effect":false,"capture":"followers","verify":false},{"expect":"profile","action":"read","target":{"id":null,"text":null,"text_any":null,"near_text_any":["正在关注","Following"]},"value":null,"side_effect":false,"capture":"following","verify":false}],"known_interrupts":[],"params":null}'::jsonb,
    'V2tDNHpA/WEfLVn0jE/R5culo7qm4vch3zJ53qK3b8o=',
    '4085d0961ea2aa991033caaaa48664e74c9507a404814e5784c85322b5192c0f',
    'active'
)
ON CONFLICT (adapter_id, version) DO UPDATE SET
    definition = EXCLUDED.definition,
    signature = EXCLUDED.signature,
    sha256 = EXCLUDED.sha256,
    state = EXCLUDED.state;

-- 2) 互动子循环 capability 行 (设备路由; 锚点需真机核验)
INSERT INTO channel_capabilities (platform, action, route, status, priority, constraints)
VALUES
    ('x', 'read_mentions', 'device', 'verified', 50, '{"adapter":"x.read_mentions"}'::jsonb),
    ('x', 'read_timeline', 'device', 'verified', 50, '{"adapter":"x.read_timeline"}'::jsonb),
    ('x', 'follow',         'device', 'verified', 50, '{"adapter":"x.follow"}'::jsonb)
ON CONFLICT (platform, action, route) DO UPDATE SET
    status = EXCLUDED.status, constraints = EXCLUDED.constraints, updated_at = now();

-- 3) 互动记忆表: read_mentions/read_timeline 抓到的提及与待回复对象 (后台子循环消费)
CREATE TABLE IF NOT EXISTS social_mentions (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id     UUID NOT NULL,
    employee_id   UUID,
    platform      TEXT NOT NULL,
    account_id    TEXT NOT NULL,
    author_handle TEXT NOT NULL,
    post_id       TEXT,
    text          TEXT NOT NULL DEFAULT '',
    kind          TEXT NOT NULL DEFAULT 'mention' CHECK (kind IN ('mention','timeline')),
    replied       BOOLEAN NOT NULL DEFAULT false,
    task_id       UUID,
    observed_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_social_mentions_pending
    ON social_mentions (platform, account_id, replied, observed_at DESC);
-- harvest 去重: 有 post_id 按 post_id, 否则按 handle+文本前缀
CREATE UNIQUE INDEX IF NOT EXISTS uq_social_mentions_post
    ON social_mentions (platform, account_id, post_id) WHERE post_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uq_social_mentions_text
    ON social_mentions (platform, account_id, author_handle, left(text, 40)) WHERE post_id IS NULL;
-- 后台循环表口径: 不单独上 FORCE RLS, 显式 agency_id/employee_id scope (同 cycle_measurements)
