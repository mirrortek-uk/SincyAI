-- 0021_skills: Skill 生态 (DE guide: skills 可版本化、可测试、动态注册, 绑定到员工)
-- 9 个 Skill 域, 初始 77 个 skills; agent 按域动态调用, skill 可复用多个 tool。

BEGIN;

-- skill 注册表 (平台级, 类似 intelligence_sources; 当前版本在 skills, 历史版本进 skill_versions)
CREATE TABLE IF NOT EXISTS skills (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    domain      TEXT NOT NULL CHECK (domain IN ('research','strategy','creative','content','quality_compliance','publishing','analytics','learning','memory')),
    name        TEXT NOT NULL UNIQUE,          -- 稳定 skill id, 如 social_post_generation
    version     INTEGER NOT NULL DEFAULT 1,
    title       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    input_schema  JSONB NOT NULL DEFAULT '{}'::jsonb,
    output_schema JSONB NOT NULL DEFAULT '{}'::jsonb,
    tools       JSONB NOT NULL DEFAULT '[]'::jsonb,   -- 该 skill 可调用的 tool 列表
    status      TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','deprecated')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS skills_domain_idx ON skills (domain);

-- skill 版本历史 (每次注册新版本留档, 可回滚/可测试)
CREATE TABLE IF NOT EXISTS skill_versions (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    skill_name  TEXT NOT NULL REFERENCES skills (name),
    version     INTEGER NOT NULL,
    spec        JSONB NOT NULL,               -- {title,description,input_schema,output_schema,tools}
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (skill_name, version)
);

-- 员工绑定的 skills (租户隔离, 经 digital_employees)
CREATE TABLE IF NOT EXISTS employee_skill_bindings (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    employee_id UUID NOT NULL REFERENCES digital_employees (id),
    skill_id    UUID NOT NULL REFERENCES skills (id),
    enabled     BOOLEAN NOT NULL DEFAULT true,
    bound_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (employee_id, skill_id)
);
ALTER TABLE employee_skill_bindings ENABLE ROW LEVEL SECURITY;
ALTER TABLE employee_skill_bindings FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON employee_skill_bindings;
CREATE POLICY tenant_isolation ON employee_skill_bindings
    USING (EXISTS (SELECT 1 FROM digital_employees e WHERE e.id = employee_id AND e.agency_id::text = current_setting('app.current_agency_id', true)))
    WITH CHECK (EXISTS (SELECT 1 FROM digital_employees e WHERE e.id = employee_id AND e.agency_id::text = current_setting('app.current_agency_id', true)));
CREATE INDEX IF NOT EXISTS employee_skill_bindings_employee_idx ON employee_skill_bindings (employee_id);

-- ============ 初始 Skill Catalog 种子 (9 域 / 77 个) ============

-- research (研究 Research)
INSERT INTO skills (domain, name, title, description, tools) VALUES ('research', 'social_search', '社交媒体搜索', '社交媒体搜索 — 研究 Research域 skill', '["reach-mcp","social-media-research-cli","TikHub","SocialDataX"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('social_search', 1, jsonb_build_object('title', '社交媒体搜索', 'description', '社交媒体搜索 — 研究 Research域 skill', 'tools', '["reach-mcp","social-media-research-cli","TikHub","SocialDataX"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('research', 'trend_discovery', '趋势发现', '趋势发现 — 研究 Research域 skill', '["TrendRadar","HotRank Hub","974.cn","xbangdan","xpaiming","SearXNG"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('trend_discovery', 1, jsonb_build_object('title', '趋势发现', 'description', '趋势发现 — 研究 Research域 skill', 'tools', '["TrendRadar","HotRank Hub","974.cn","xbangdan","xpaiming","SearXNG"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('research', 'topic_research', '话题研究', '话题研究 — 研究 Research域 skill', '["SearXNG","Firecrawl","Crawl4AI","RSSHub","Miniflux"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('topic_research', 1, jsonb_build_object('title', '话题研究', 'description', '话题研究 — 研究 Research域 skill', 'tools', '["SearXNG","Firecrawl","Crawl4AI","RSSHub","Miniflux"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('research', 'audience_research', '受众研究', '受众研究 — 研究 Research域 skill', '["social_search","sentiment_analysis"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('audience_research', 1, jsonb_build_object('title', '受众研究', 'description', '受众研究 — 研究 Research域 skill', 'tools', '["social_search","sentiment_analysis"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('research', 'competitor_analysis', '竞品分析', '竞品分析 — 研究 Research域 skill', '["social_search","trend_discovery"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('competitor_analysis', 1, jsonb_build_object('title', '竞品分析', 'description', '竞品分析 — 研究 Research域 skill', 'tools', '["social_search","trend_discovery"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('research', 'creator_analysis', '创作者分析', '创作者分析 — 研究 Research域 skill', '["social_search","reach-mcp"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('creator_analysis', 1, jsonb_build_object('title', '创作者分析', 'description', '创作者分析 — 研究 Research域 skill', 'tools', '["social_search","reach-mcp"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('research', 'content_gap_analysis', '内容缺口分析', '内容缺口分析 — 研究 Research域 skill', '["trend_discovery","social_search"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('content_gap_analysis', 1, jsonb_build_object('title', '内容缺口分析', 'description', '内容缺口分析 — 研究 Research域 skill', 'tools', '["trend_discovery","social_search"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('research', 'source_verification', '信源核验', '信源核验 — 研究 Research域 skill', '["SearXNG","Firecrawl"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('source_verification', 1, jsonb_build_object('title', '信源核验', 'description', '信源核验 — 研究 Research域 skill', 'tools', '["SearXNG","Firecrawl"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('research', 'fact_checking', '事实核查', '事实核查 — 研究 Research域 skill', '["web_research","source_verification"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('fact_checking', 1, jsonb_build_object('title', '事实核查', 'description', '事实核查 — 研究 Research域 skill', 'tools', '["web_research","source_verification"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('research', 'web_research', '网页研究', '网页研究 — 研究 Research域 skill', '["Firecrawl","Crawl4AI","SearXNG"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('web_research', 1, jsonb_build_object('title', '网页研究', 'description', '网页研究 — 研究 Research域 skill', 'tools', '["Firecrawl","Crawl4AI","SearXNG"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;

-- strategy (战略 Strategy)
INSERT INTO skills (domain, name, title, description, tools) VALUES ('strategy', 'content_strategy', '内容战略', '内容战略 — 战略 Strategy域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('content_strategy', 1, jsonb_build_object('title', '内容战略', 'description', '内容战略 — 战略 Strategy域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('strategy', 'audience_strategy', '受众战略', '受众战略 — 战略 Strategy域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('audience_strategy', 1, jsonb_build_object('title', '受众战略', 'description', '受众战略 — 战略 Strategy域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('strategy', 'channel_strategy', '渠道战略', '渠道战略 — 战略 Strategy域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('channel_strategy', 1, jsonb_build_object('title', '渠道战略', 'description', '渠道战略 — 战略 Strategy域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('strategy', 'content_pillar_planning', '内容支柱规划', '内容支柱规划 — 战略 Strategy域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('content_pillar_planning', 1, jsonb_build_object('title', '内容支柱规划', 'description', '内容支柱规划 — 战略 Strategy域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('strategy', 'campaign_planning', ' campaign 活动策划', ' campaign 活动策划 — 战略 Strategy域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('campaign_planning', 1, jsonb_build_object('title', ' campaign 活动策划', 'description', ' campaign 活动策划 — 战略 Strategy域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('strategy', 'growth_strategy', '增长战略', '增长战略 — 战略 Strategy域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('growth_strategy', 1, jsonb_build_object('title', '增长战略', 'description', '增长战略 — 战略 Strategy域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('strategy', 'competitor_positioning', '竞品定位', '竞品定位 — 战略 Strategy域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('competitor_positioning', 1, jsonb_build_object('title', '竞品定位', 'description', '竞品定位 — 战略 Strategy域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;

-- creative (创意 Creative)
INSERT INTO skills (domain, name, title, description, tools) VALUES ('creative', 'idea_generation', '创意生成', '创意生成 — 创意 Creative域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('idea_generation', 1, jsonb_build_object('title', '创意生成', 'description', '创意生成 — 创意 Creative域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('creative', 'angle_discovery', '角度发现', '角度发现 — 创意 Creative域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('angle_discovery', 1, jsonb_build_object('title', '角度发现', 'description', '角度发现 — 创意 Creative域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('creative', 'hook_generation', 'Hook 生成', 'Hook 生成 — 创意 Creative域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('hook_generation', 1, jsonb_build_object('title', 'Hook 生成', 'description', 'Hook 生成 — 创意 Creative域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('creative', 'storytelling', '故事化叙事', '故事化叙事 — 创意 Creative域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('storytelling', 1, jsonb_build_object('title', '故事化叙事', 'description', '故事化叙事 — 创意 Creative域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('creative', 'viral_format_analysis', '爆款格式分析', '爆款格式分析 — 创意 Creative域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('viral_format_analysis', 1, jsonb_build_object('title', '爆款格式分析', 'description', '爆款格式分析 — 创意 Creative域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('creative', 'creative_variation', '创意变体', '创意变体 — 创意 Creative域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('creative_variation', 1, jsonb_build_object('title', '创意变体', 'description', '创意变体 — 创意 Creative域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('creative', 'content_brainstorming', '内容头脑风暴', '内容头脑风暴 — 创意 Creative域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('content_brainstorming', 1, jsonb_build_object('title', '内容头脑风暴', 'description', '内容头脑风暴 — 创意 Creative域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('creative', 'trend_to_content', '趋势转内容', '趋势转内容 — 创意 Creative域 skill', '["trend_discovery"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('trend_to_content', 1, jsonb_build_object('title', '趋势转内容', 'description', '趋势转内容 — 创意 Creative域 skill', 'tools', '["trend_discovery"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;

-- content (内容生产 Content Production)
INSERT INTO skills (domain, name, title, description, tools) VALUES ('content', 'text_generation', '文本生成', '文本生成 — 内容生产 Content Production域 skill', '["llm"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('text_generation', 1, jsonb_build_object('title', '文本生成', 'description', '文本生成 — 内容生产 Content Production域 skill', 'tools', '["llm"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('content', 'article_writing', '文章写作', '文章写作 — 内容生产 Content Production域 skill', '["llm"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('article_writing', 1, jsonb_build_object('title', '文章写作', 'description', '文章写作 — 内容生产 Content Production域 skill', 'tools', '["llm"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('content', 'social_post_generation', '社交帖子生成', '社交帖子生成 — 内容生产 Content Production域 skill', '["llm"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('social_post_generation', 1, jsonb_build_object('title', '社交帖子生成', 'description', '社交帖子生成 — 内容生产 Content Production域 skill', 'tools', '["llm"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('content', 'script_generation', '脚本生成', '脚本生成 — 内容生产 Content Production域 skill', '["llm"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('script_generation', 1, jsonb_build_object('title', '脚本生成', 'description', '脚本生成 — 内容生产 Content Production域 skill', 'tools', '["llm"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('content', 'image_generation', '图片生成', '图片生成 — 内容生产 Content Production域 skill', '["image_model"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('image_generation', 1, jsonb_build_object('title', '图片生成', 'description', '图片生成 — 内容生产 Content Production域 skill', 'tools', '["image_model"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('content', 'image_editing', '图片编辑', '图片编辑 — 内容生产 Content Production域 skill', '["image_model"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('image_editing', 1, jsonb_build_object('title', '图片编辑', 'description', '图片编辑 — 内容生产 Content Production域 skill', 'tools', '["image_model"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('content', 'video_generation', '视频生成', '视频生成 — 内容生产 Content Production域 skill', '["video_model"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('video_generation', 1, jsonb_build_object('title', '视频生成', 'description', '视频生成 — 内容生产 Content Production域 skill', 'tools', '["video_model"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('content', 'video_editing', '视频剪辑', '视频剪辑 — 内容生产 Content Production域 skill', '["video_model"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('video_editing', 1, jsonb_build_object('title', '视频剪辑', 'description', '视频剪辑 — 内容生产 Content Production域 skill', 'tools', '["video_model"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('content', 'caption_generation', '配文生成', '配文生成 — 内容生产 Content Production域 skill', '["llm"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('caption_generation', 1, jsonb_build_object('title', '配文生成', 'description', '配文生成 — 内容生产 Content Production域 skill', 'tools', '["llm"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('content', 'subtitle_generation', '字幕生成', '字幕生成 — 内容生产 Content Production域 skill', '["llm"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('subtitle_generation', 1, jsonb_build_object('title', '字幕生成', 'description', '字幕生成 — 内容生产 Content Production域 skill', 'tools', '["llm"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('content', 'carousel_generation', '轮播图生成', '轮播图生成 — 内容生产 Content Production域 skill', '["llm","image_model"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('carousel_generation', 1, jsonb_build_object('title', '轮播图生成', 'description', '轮播图生成 — 内容生产 Content Production域 skill', 'tools', '["llm","image_model"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('content', 'content_rewriting', '内容改写', '内容改写 — 内容生产 Content Production域 skill', '["llm"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('content_rewriting', 1, jsonb_build_object('title', '内容改写', 'description', '内容改写 — 内容生产 Content Production域 skill', 'tools', '["llm"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('content', 'localization', '本地化', '本地化 — 内容生产 Content Production域 skill', '["llm"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('localization', 1, jsonb_build_object('title', '本地化', 'description', '本地化 — 内容生产 Content Production域 skill', 'tools', '["llm"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('content', 'translation', '翻译', '翻译 — 内容生产 Content Production域 skill', '["llm"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('translation', 1, jsonb_build_object('title', '翻译', 'description', '翻译 — 内容生产 Content Production域 skill', 'tools', '["llm"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;

-- quality_compliance (质量与合规 Quality & Compliance)
INSERT INTO skills (domain, name, title, description, tools) VALUES ('quality_compliance', 'content_quality_review', '内容质量审核', '内容质量审核 — 质量与合规 Quality & Compliance域 skill', '["llm"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('content_quality_review', 1, jsonb_build_object('title', '内容质量审核', 'description', '内容质量审核 — 质量与合规 Quality & Compliance域 skill', 'tools', '["llm"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('quality_compliance', 'brand_voice_review', '品牌调性审核', '品牌调性审核 — 质量与合规 Quality & Compliance域 skill', '["llm"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('brand_voice_review', 1, jsonb_build_object('title', '品牌调性审核', 'description', '品牌调性审核 — 质量与合规 Quality & Compliance域 skill', 'tools', '["llm"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('quality_compliance', 'originality_check', '原创性检查', '原创性检查 — 质量与合规 Quality & Compliance域 skill', '["llm"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('originality_check', 1, jsonb_build_object('title', '原创性检查', 'description', '原创性检查 — 质量与合规 Quality & Compliance域 skill', 'tools', '["llm"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('quality_compliance', 'fact_check', '事实核查', '事实核查 — 质量与合规 Quality & Compliance域 skill', '["fact_checking"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('fact_check', 1, jsonb_build_object('title', '事实核查', 'description', '事实核查 — 质量与合规 Quality & Compliance域 skill', 'tools', '["fact_checking"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('quality_compliance', 'copyright_check', '版权检查', '版权检查 — 质量与合规 Quality & Compliance域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('copyright_check', 1, jsonb_build_object('title', '版权检查', 'description', '版权检查 — 质量与合规 Quality & Compliance域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('quality_compliance', 'platform_compliance', '平台合规检查', '平台合规检查 — 质量与合规 Quality & Compliance域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('platform_compliance', 1, jsonb_build_object('title', '平台合规检查', 'description', '平台合规检查 — 质量与合规 Quality & Compliance域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('quality_compliance', 'safety_review', '安全审核', '安全审核 — 质量与合规 Quality & Compliance域 skill', '["llm"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('safety_review', 1, jsonb_build_object('title', '安全审核', 'description', '安全审核 — 质量与合规 Quality & Compliance域 skill', 'tools', '["llm"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('quality_compliance', 'claim_verification', '声明核实', '声明核实 — 质量与合规 Quality & Compliance域 skill', '["fact_checking"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('claim_verification', 1, jsonb_build_object('title', '声明核实', 'description', '声明核实 — 质量与合规 Quality & Compliance域 skill', 'tools', '["fact_checking"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('quality_compliance', 'content_scoring', '内容打分', '内容打分 — 质量与合规 Quality & Compliance域 skill', '["llm"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('content_scoring', 1, jsonb_build_object('title', '内容打分', 'description', '内容打分 — 质量与合规 Quality & Compliance域 skill', 'tools', '["llm"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;

-- publishing (发布 Publishing)
INSERT INTO skills (domain, name, title, description, tools) VALUES ('publishing', 'platform_formatting', '平台格式化', '平台格式化 — 发布 Publishing域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('platform_formatting', 1, jsonb_build_object('title', '平台格式化', 'description', '平台格式化 — 发布 Publishing域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('publishing', 'content_scheduling', '内容排期', '内容排期 — 发布 Publishing域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('content_scheduling', 1, jsonb_build_object('title', '内容排期', 'description', '内容排期 — 发布 Publishing域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('publishing', 'content_publishing', '内容发布', '内容发布 — 发布 Publishing域 skill', '["device_execution"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('content_publishing', 1, jsonb_build_object('title', '内容发布', 'description', '内容发布 — 发布 Publishing域 skill', 'tools', '["device_execution"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('publishing', 'account_selection', '账号选择', '账号选择 — 发布 Publishing域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('account_selection', 1, jsonb_build_object('title', '账号选择', 'description', '账号选择 — 发布 Publishing域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('publishing', 'media_upload', '媒体上传', '媒体上传 — 发布 Publishing域 skill', '["device_execution"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('media_upload', 1, jsonb_build_object('title', '媒体上传', 'description', '媒体上传 — 发布 Publishing域 skill', 'tools', '["device_execution"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('publishing', 'publishing_verification', '发布验证', '发布验证 — 发布 Publishing域 skill', '["device_execution"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('publishing_verification', 1, jsonb_build_object('title', '发布验证', 'description', '发布验证 — 发布 Publishing域 skill', 'tools', '["device_execution"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('publishing', 'retry_management', '重试管理', '重试管理 — 发布 Publishing域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('retry_management', 1, jsonb_build_object('title', '重试管理', 'description', '重试管理 — 发布 Publishing域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;

-- analytics (数据分析 Analytics)
INSERT INTO skills (domain, name, title, description, tools) VALUES ('analytics', 'performance_analysis', '表现分析', '表现分析 — 数据分析 Analytics域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('performance_analysis', 1, jsonb_build_object('title', '表现分析', 'description', '表现分析 — 数据分析 Analytics域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('analytics', 'engagement_analysis', '互动分析', '互动分析 — 数据分析 Analytics域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('engagement_analysis', 1, jsonb_build_object('title', '互动分析', 'description', '互动分析 — 数据分析 Analytics域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('analytics', 'audience_analysis', '受众分析', '受众分析 — 数据分析 Analytics域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('audience_analysis', 1, jsonb_build_object('title', '受众分析', 'description', '受众分析 — 数据分析 Analytics域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('analytics', 'viral_content_analysis', '爆款内容分析', '爆款内容分析 — 数据分析 Analytics域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('viral_content_analysis', 1, jsonb_build_object('title', '爆款内容分析', 'description', '爆款内容分析 — 数据分析 Analytics域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('analytics', 'content_comparison', '内容对比', '内容对比 — 数据分析 Analytics域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('content_comparison', 1, jsonb_build_object('title', '内容对比', 'description', '内容对比 — 数据分析 Analytics域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('analytics', 'trend_performance_analysis', '趋势表现分析', '趋势表现分析 — 数据分析 Analytics域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('trend_performance_analysis', 1, jsonb_build_object('title', '趋势表现分析', 'description', '趋势表现分析 — 数据分析 Analytics域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('analytics', 'campaign_analysis', '活动分析', '活动分析 — 数据分析 Analytics域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('campaign_analysis', 1, jsonb_build_object('title', '活动分析', 'description', '活动分析 — 数据分析 Analytics域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;

-- learning (学习与优化 Learning)
INSERT INTO skills (domain, name, title, description, tools) VALUES ('learning', 'insight_extraction', '洞察提取', '洞察提取 — 学习与优化 Learning域 skill', '["llm"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('insight_extraction', 1, jsonb_build_object('title', '洞察提取', 'description', '洞察提取 — 学习与优化 Learning域 skill', 'tools', '["llm"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('learning', 'pattern_detection', '模式检测', '模式检测 — 学习与优化 Learning域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('pattern_detection', 1, jsonb_build_object('title', '模式检测', 'description', '模式检测 — 学习与优化 Learning域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('learning', 'experiment_design', '实验设计', '实验设计 — 学习与优化 Learning域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('experiment_design', 1, jsonb_build_object('title', '实验设计', 'description', '实验设计 — 学习与优化 Learning域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('learning', 'ab_test_analysis', 'A/B 分析', 'A/B 分析 — 学习与优化 Learning域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('ab_test_analysis', 1, jsonb_build_object('title', 'A/B 分析', 'description', 'A/B 分析 — 学习与优化 Learning域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('learning', 'strategy_optimization', '策略优化', '策略优化 — 学习与优化 Learning域 skill', '["llm"]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('strategy_optimization', 1, jsonb_build_object('title', '策略优化', 'description', '策略优化 — 学习与优化 Learning域 skill', 'tools', '["llm"]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('learning', 'creative_dna_update', '创意 DNA 更新', '创意 DNA 更新 — 学习与优化 Learning域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('creative_dna_update', 1, jsonb_build_object('title', '创意 DNA 更新', 'description', '创意 DNA 更新 — 学习与优化 Learning域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('learning', 'memory_update', '记忆更新', '记忆更新 — 学习与优化 Learning域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('memory_update', 1, jsonb_build_object('title', '记忆更新', 'description', '记忆更新 — 学习与优化 Learning域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('learning', 'skill_evaluation', '技能评估', '技能评估 — 学习与优化 Learning域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('skill_evaluation', 1, jsonb_build_object('title', '技能评估', 'description', '技能评估 — 学习与优化 Learning域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;

-- memory (记忆管理 Memory)
INSERT INTO skills (domain, name, title, description, tools) VALUES ('memory', 'episodic_memory', '情景记忆', '情景记忆 — 记忆管理 Memory域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('episodic_memory', 1, jsonb_build_object('title', '情景记忆', 'description', '情景记忆 — 记忆管理 Memory域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('memory', 'semantic_memory', '语义记忆', '语义记忆 — 记忆管理 Memory域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('semantic_memory', 1, jsonb_build_object('title', '语义记忆', 'description', '语义记忆 — 记忆管理 Memory域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('memory', 'audience_memory', '受众记忆', '受众记忆 — 记忆管理 Memory域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('audience_memory', 1, jsonb_build_object('title', '受众记忆', 'description', '受众记忆 — 记忆管理 Memory域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('memory', 'brand_memory', '品牌记忆', '品牌记忆 — 记忆管理 Memory域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('brand_memory', 1, jsonb_build_object('title', '品牌记忆', 'description', '品牌记忆 — 记忆管理 Memory域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('memory', 'creative_memory', '创意记忆', '创意记忆 — 记忆管理 Memory域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('creative_memory', 1, jsonb_build_object('title', '创意记忆', 'description', '创意记忆 — 记忆管理 Memory域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('memory', 'performance_memory', '表现记忆', '表现记忆 — 记忆管理 Memory域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('performance_memory', 1, jsonb_build_object('title', '表现记忆', 'description', '表现记忆 — 记忆管理 Memory域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;
INSERT INTO skills (domain, name, title, description, tools) VALUES ('memory', 'experiment_memory', '实验记忆', '实验记忆 — 记忆管理 Memory域 skill', '[]'::jsonb) ON CONFLICT (name) DO NOTHING;
INSERT INTO skill_versions (skill_name, version, spec) VALUES ('experiment_memory', 1, jsonb_build_object('title', '实验记忆', 'description', '实验记忆 — 记忆管理 Memory域 skill', 'tools', '[]'::jsonb)) ON CONFLICT (skill_name, version) DO NOTHING;

COMMIT;

-- seeded 77 skills in 9 domains