-- 0046: 全平台 KPI / 配额规格 (claude/kpi-schema.md v1)
-- kpi_categories(6 档) / platform_kpi_templates(平台画像) / account_kpi(账号覆盖 merge 模板 = effective)
-- account_activity_ledger 记录每日动作消耗, 配额压顶检查据它做。
-- 口径: 后台循环涉及的表不单独上 FORCE RLS, 显式 agency_id scope (同 0043/0044)。

CREATE TABLE IF NOT EXISTS kpi_categories (
    category           TEXT PRIMARY KEY,
    label              TEXT NOT NULL,
    growth_metric      TEXT NOT NULL,
    engagement_formula TEXT NOT NULL,
    available_actions  TEXT[] NOT NULL,
    default_quotas     JSONB NOT NULL,
    quality_gates      JSONB NOT NULL,
    health_guardrails  JSONB NOT NULL,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS platform_kpi_templates (
    platform             TEXT PRIMARY KEY,
    label                TEXT NOT NULL,
    category             TEXT NOT NULL REFERENCES kpi_categories (category),
    growth_metric        TEXT NOT NULL,
    engagement_formula   TEXT NOT NULL,
    available_actions    TEXT[] NOT NULL,
    default_quotas       JSONB NOT NULL,
    secondary_metrics    TEXT[] NOT NULL DEFAULT '{}',
    health_signals       TEXT[] NOT NULL DEFAULT '{}',
    rate_caps            JSONB NOT NULL,
    adapter_requirements TEXT[] NOT NULL DEFAULT '{}',
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS account_kpi (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agency_id   UUID NOT NULL REFERENCES agencies (id),
    employee_id UUID REFERENCES digital_employees (id),
    platform    TEXT NOT NULL REFERENCES platform_kpi_templates (platform),
    account_id  TEXT NOT NULL,
    overrides   JSONB NOT NULL DEFAULT '{}'::jsonb,
    effective   JSONB NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (agency_id, platform, account_id)
);

CREATE TABLE IF NOT EXISTS account_activity_ledger (
    id          BIGSERIAL PRIMARY KEY,
    agency_id   UUID NOT NULL,
    employee_id UUID,
    platform    TEXT NOT NULL,
    account_id  TEXT NOT NULL,
    date        DATE NOT NULL DEFAULT CURRENT_DATE,
    action      TEXT NOT NULL,
    count       INTEGER NOT NULL DEFAULT 0,
    UNIQUE (platform, account_id, date, action)
);
CREATE INDEX IF NOT EXISTS idx_ledger_today ON account_activity_ledger (platform, account_id, date);

INSERT INTO kpi_categories (category, label, growth_metric, engagement_formula, available_actions, default_quotas, quality_gates, health_guardrails) VALUES
('text_social', '文本社交', 'followers', '(likes+replies+reposts)/impressions', ARRAY['post','reply','repost','quote','like','follow','dm']::text[], '{"posts_per_day": 3, "replies_per_day": 12, "follows_per_day": 15, "likes_per_day": 30, "reposts_per_day": 3, "dms_per_day": 0}'::jsonb, '{"min_quality_score": 60, "require_fact_check_pass": true, "max_duplicate_ratio": 0.2, "forbidden_topics_enforced": true, "ai_label_required": true}'::jsonb, '{"max_actions_per_hour": 20, "daily_action_ceiling": 80, "stop_on_shadowban": true, "stop_on_rate_limit": true, "stop_on_policy_strike": true, "cooldown_minutes_after_block": 120, "challenge_to_human": true}'::jsonb),
('pro_social', '职业社交', 'followers', '(reacts+comments+reposts)/impressions', ARRAY['post','comment','react','connect','follow']::text[], '{"posts_per_week": 4, "comments_per_day": 8, "connects_per_day": 12, "reacts_per_day": 20, "follows_per_day": 10}'::jsonb, '{"min_quality_score": 60, "require_fact_check_pass": true, "max_duplicate_ratio": 0.2, "forbidden_topics_enforced": true, "ai_label_required": true}'::jsonb, '{"max_actions_per_hour": 15, "daily_action_ceiling": 60, "stop_on_shadowban": true, "stop_on_rate_limit": true, "stop_on_policy_strike": true, "cooldown_minutes_after_block": 120, "challenge_to_human": true}'::jsonb),
('image_lifestyle', '图文生活', 'followers', '(likes+comments+saves+shares)/impressions', ARRAY['post','comment','like','save','follow','dm']::text[], '{"posts_per_day": 1, "comments_per_day": 12, "likes_per_day": 25, "follows_per_day": 15, "saves_per_day": 10, "dms_per_day": 0}'::jsonb, '{"min_quality_score": 60, "require_fact_check_pass": true, "max_duplicate_ratio": 0.2, "forbidden_topics_enforced": true, "ai_label_required": true}'::jsonb, '{"max_actions_per_hour": 18, "daily_action_ceiling": 70, "stop_on_shadowban": true, "stop_on_rate_limit": true, "stop_on_policy_strike": true, "cooldown_minutes_after_block": 120, "challenge_to_human": true}'::jsonb),
('short_video', '短视频', 'followers', '(likes+comments+shares+saves)/views + completion_rate', ARRAY['post','comment','like','follow','duet','stitch','share']::text[], '{"posts_per_day": 1, "comments_per_day": 15, "likes_per_day": 30, "follows_per_day": 15, "duet_stitch_per_day": 1}'::jsonb, '{"min_quality_score": 60, "require_fact_check_pass": true, "max_duplicate_ratio": 0.2, "forbidden_topics_enforced": true, "ai_label_required": true}'::jsonb, '{"max_actions_per_hour": 15, "daily_action_ceiling": 60, "stop_on_shadowban": true, "stop_on_rate_limit": true, "stop_on_policy_strike": true, "cooldown_minutes_after_block": 120, "challenge_to_human": true}'::jsonb),
('long_video', '长视频', 'subscribers', 'watch_time + retention + CTR + (likes+comments)', ARRAY['publish','community_post','comment','reply','like']::text[], '{"posts_per_week": 3, "community_posts_per_day": 1, "comments_per_day": 15, "likes_per_day": 15}'::jsonb, '{"min_quality_score": 60, "require_fact_check_pass": true, "max_duplicate_ratio": 0.2, "forbidden_topics_enforced": true, "ai_label_required": true}'::jsonb, '{"max_actions_per_hour": 10, "daily_action_ceiling": 40, "stop_on_shadowban": true, "stop_on_rate_limit": true, "stop_on_policy_strike": true, "cooldown_minutes_after_block": 120, "challenge_to_human": true}'::jsonb),
('community', '社区', 'karma', 'upvote_ratio + comment_karma', ARRAY['post','comment','upvote','crosspost']::text[], '{"posts_per_day": 1, "comments_per_day": 10, "upvotes_per_day": 20}'::jsonb, '{"min_quality_score": 60, "require_fact_check_pass": true, "max_duplicate_ratio": 0.2, "forbidden_topics_enforced": true, "ai_label_required": true}'::jsonb, '{"max_actions_per_hour": 10, "daily_action_ceiling": 40, "stop_on_shadowban": true, "stop_on_rate_limit": true, "stop_on_policy_strike": true, "cooldown_minutes_after_block": 120, "challenge_to_human": true}'::jsonb)
ON CONFLICT (category) DO UPDATE SET label=EXCLUDED.label, default_quotas=EXCLUDED.default_quotas, quality_gates=EXCLUDED.quality_gates, health_guardrails=EXCLUDED.health_guardrails, available_actions=EXCLUDED.available_actions, growth_metric=EXCLUDED.growth_metric, engagement_formula=EXCLUDED.engagement_formula, updated_at=now();

INSERT INTO platform_kpi_templates (platform, label, category, growth_metric, engagement_formula, available_actions, default_quotas, secondary_metrics, health_signals, rate_caps, adapter_requirements) VALUES
('x', 'X (Twitter)', 'text_social', 'followers', '(likes+replies+reposts)/impressions', ARRAY['post','reply','repost','quote','like','follow','dm']::text[], '{"posts_per_day": 3, "replies_per_day": 12, "follows_per_day": 15, "likes_per_day": 30, "reposts_per_day": 3, "quotes_per_day": 1, "dms_per_day": 0}'::jsonb, ARRAY['tweet_impressions','profile_visits']::text[], ARRAY['shadowban','reply_visibility','rate_limit_429']::text[], '{"max_actions_per_hour": 20, "daily_action_ceiling": 80}'::jsonb, ARRAY['dm','follow','like','post','quote','read_followers','read_mentions','reply','repost']::text[]),
('xhs', '小红书', 'image_lifestyle', 'followers', '(likes+comments+saves+shares)/impressions', ARRAY['post','comment','like','save','follow','dm']::text[], '{"posts_per_day": 1, "comments_per_day": 12, "follows_per_day": 15, "likes_per_day": 25, "dms_per_day": 0, "saves_per_day": null}'::jsonb, ARRAY['收藏率','推荐池比例','搜索排名']::text[], ARRAY['rate_limit','policy_violation','note_folded','dm_throttle']::text[], '{"max_actions_per_hour": 18, "daily_action_ceiling": 70}'::jsonb, ARRAY['comment','dm','follow','like','post','read_followers','read_mentions','save']::text[]),
('tiktok', 'TikTok', 'short_video', 'followers', '(likes+comments+shares+saves)/views + completion_rate', ARRAY['post','comment','like','follow','duet','stitch','share']::text[], '{"posts_per_day": 1, "comments_per_day": 15, "follows_per_day": 15, "likes_per_day": 30, "duet_stitch_per_day": 1}'::jsonb, ARRAY['completion_rate','avg_watch_time','fyp_entry','shares']::text[], ARRAY['low_fyp_traffic','community_violation','low_completion']::text[], '{"max_actions_per_hour": 15, "daily_action_ceiling": 60}'::jsonb, ARRAY['comment','duet','follow','like','post','read_followers','read_mentions','share','stitch']::text[]),
('youtube', 'YouTube', 'long_video', 'subscribers', 'watch_time + retention + CTR + (likes+comments)', ARRAY['publish','community_post','comment','reply','like']::text[], '{"posts_per_day": null, "posts_per_week": 3, "community_posts_per_day": 1, "comments_per_day": 15, "replies_per_day": 15, "likes_per_day": 15}'::jsonb, ARRAY['ctr','avg_view_duration','retention','sub_conversion']::text[], ARRAY['community_strike','copyright','rate_limit']::text[], '{"max_actions_per_hour": 10, "daily_action_ceiling": 40}'::jsonb, ARRAY['comment','community_post','like','publish','read_followers','read_mentions','reply']::text[]),
('instagram', 'Instagram', 'image_lifestyle', 'followers', '(likes+comments+saves+shares)/impressions', ARRAY['post','comment','like','save','follow','dm']::text[], '{"posts_per_day": 1, "reels_per_day": 1, "stories_per_day": 3, "comments_per_day": 12, "follows_per_day": 15, "likes_per_day": 30, "dms_per_day": 0}'::jsonb, ARRAY['saves','reach','profile_visits','reels_completion']::text[], ARRAY['action_block','shadowban_tags','policy_violation']::text[], '{"max_actions_per_hour": 18, "daily_action_ceiling": 70}'::jsonb, ARRAY['comment','dm','follow','like','post','read_followers','read_mentions','save']::text[]),
('weibo', '微博', 'text_social', 'followers', '(likes+replies+reposts)/impressions', ARRAY['post','reply','repost','quote','like','follow','dm']::text[], '{"posts_per_day": 3, "comments_per_day": 12, "follows_per_day": 15, "likes_per_day": 30, "reposts_per_day": 3}'::jsonb, ARRAY['阅读数','转发链','热搜话题']::text[], ARRAY['rate_limit','mute','follow_throttle']::text[], '{"max_actions_per_hour": 20, "daily_action_ceiling": 80}'::jsonb, ARRAY['dm','follow','like','post','quote','read_followers','read_mentions','reply','repost']::text[]),
('douyin', '抖音', 'short_video', 'followers', '(likes+comments+shares+saves)/views + completion_rate', ARRAY['post','comment','like','follow','duet','stitch','share']::text[], '{"posts_per_day": 1, "comments_per_day": 15, "follows_per_day": 15, "likes_per_day": 30, "dms_per_day": 0}'::jsonb, ARRAY['完播率','平均时长','推荐池进阶','分享']::text[], ARRAY['rate_limit','violation','搬运检测','low_completion']::text[], '{"max_actions_per_hour": 15, "daily_action_ceiling": 60}'::jsonb, ARRAY['comment','duet','follow','like','post','read_followers','read_mentions','share','stitch']::text[]),
('kuaishou', '快手', 'short_video', 'followers', '(likes+comments+shares+saves)/views + completion_rate', ARRAY['post','comment','like','follow','duet','stitch','share']::text[], '{"posts_per_day": 1, "comments_per_day": 20, "follows_per_day": 15, "likes_per_day": 30}'::jsonb, ARRAY['完播率','老铁互动','评论区经营']::text[], ARRAY['rate_limit','violation','low_completion']::text[], '{"max_actions_per_hour": 15, "daily_action_ceiling": 60}'::jsonb, ARRAY['comment','duet','follow','like','post','read_followers','read_mentions','share','stitch']::text[]),
('bilibili', 'Bilibili', 'long_video', 'subscribers', 'watch_time + retention + CTR + (likes+comments)', ARRAY['publish','community_post','comment','reply','like']::text[], '{"posts_per_day": null, "posts_per_week": 3, "dynamics_per_day": 1, "comments_per_day": 15, "follows_per_day": 12, "likes_per_day": 15}'::jsonb, ARRAY['三连率','完播','弹幕密度','追番转化']::text[], ARRAY['violation','rate_limit','搬运检测']::text[], '{"max_actions_per_hour": 10, "daily_action_ceiling": 40}'::jsonb, ARRAY['comment','community_post','like','publish','read_followers','read_mentions','reply']::text[]),
('linkedin', 'LinkedIn', 'pro_social', 'followers', '(reacts+comments+reposts)/impressions', ARRAY['post','comment','react','connect','follow']::text[], '{"posts_per_day": null, "posts_per_week": 4, "comments_per_day": 8, "connects_per_day": 12, "reacts_per_day": 20}'::jsonb, ARRAY['impressions','profile_views','connect_accept_rate']::text[], ARRAY['weekly_connect_cap_100','restricted_review']::text[], '{"max_actions_per_hour": 15, "daily_action_ceiling": 60}'::jsonb, ARRAY['comment','connect','follow','post','react','read_followers','read_mentions']::text[]),
('threads', 'Threads', 'text_social', 'followers', '(likes+replies+reposts)/impressions', ARRAY['post','reply','repost','quote','like','follow','dm']::text[], '{"posts_per_day": 2, "replies_per_day": 10, "follows_per_day": 12, "likes_per_day": 25}'::jsonb, ARRAY['impressions']::text[], ARRAY['rate_limit','policy_violation']::text[], '{"max_actions_per_hour": 20, "daily_action_ceiling": 80}'::jsonb, ARRAY['dm','follow','like','post','quote','read_followers','read_mentions','reply','repost']::text[]),
('bluesky', 'Bluesky', 'text_social', 'followers', '(likes+replies+reposts)/impressions', ARRAY['post','reply','repost','quote','like','follow','dm']::text[], '{"posts_per_day": 2, "replies_per_day": 10, "follows_per_day": 12, "likes_per_day": 25, "reposts_per_day": 2}'::jsonb, ARRAY['impressions']::text[], ARRAY['rate_limit','policy_violation']::text[], '{"max_actions_per_hour": 20, "daily_action_ceiling": 80}'::jsonb, ARRAY['dm','follow','like','post','quote','read_followers','read_mentions','reply','repost']::text[]),
('mastodon', 'Mastodon', 'text_social', 'followers', '(likes+replies+reposts)/impressions', ARRAY['post','reply','repost','quote','like','follow','dm']::text[], '{"posts_per_day": 2, "replies_per_day": 10, "follows_per_day": 12, "likes_per_day": 25, "boosts_per_day": 2}'::jsonb, ARRAY['impressions']::text[], ARRAY['rate_limit','instance_block']::text[], '{"max_actions_per_hour": 20, "daily_action_ceiling": 80}'::jsonb, ARRAY['dm','follow','like','post','quote','read_followers','read_mentions','reply','repost']::text[]),
('pinterest', 'Pinterest', 'image_lifestyle', 'followers', '(likes+comments+saves+shares)/impressions', ARRAY['post','comment','like','save','follow','dm']::text[], '{"posts_per_day": 3, "repins_per_day": 5, "follows_per_day": 10, "comments_per_day": 5}'::jsonb, ARRAY['pin_impressions','saves','outbound_clicks']::text[], ARRAY['rate_limit','policy_violation']::text[], '{"max_actions_per_hour": 18, "daily_action_ceiling": 70}'::jsonb, ARRAY['comment','dm','follow','like','post','read_followers','read_mentions','save']::text[]),
('reddit', 'Reddit', 'community', 'karma', 'upvote_ratio + comment_karma', ARRAY['post','comment','upvote','crosspost']::text[], '{"posts_per_day": 1, "comments_per_day": 10, "upvotes_per_day": 20, "crossposts_per_day": 1}'::jsonb, ARRAY['upvote_ratio','comment_karma','removal_rate']::text[], ARRAY['shadowban','account_age_karma_gate','subreddit_ban','self_promo_ratio_9to1']::text[], '{"max_actions_per_hour": 10, "daily_action_ceiling": 40}'::jsonb, ARRAY['comment','crosspost','post','read_followers','read_mentions','upvote']::text[]),
('facebook', 'Facebook Page', 'text_social', 'followers', '(likes+replies+reposts)/impressions', ARRAY['post','reply','repost','quote','like','follow','dm']::text[], '{"posts_per_day": 1, "comments_per_day": 10, "likes_per_day": 20}'::jsonb, ARRAY['reach','page_views','shares']::text[], ARRAY['rate_limit','policy_violation']::text[], '{"max_actions_per_hour": 20, "daily_action_ceiling": 80}'::jsonb, ARRAY['dm','follow','like','post','quote','read_followers','read_mentions','reply','repost']::text[])
ON CONFLICT (platform) DO UPDATE SET label=EXCLUDED.label, category=EXCLUDED.category, default_quotas=EXCLUDED.default_quotas, secondary_metrics=EXCLUDED.secondary_metrics, health_signals=EXCLUDED.health_signals, rate_caps=EXCLUDED.rate_caps, adapter_requirements=EXCLUDED.adapter_requirements, updated_at=now();
