-- 0033_industry_intel: Sincy-04 行业情报与客户相关性系统 (P1 taxonomy + P2 分类 + P3画像 + 内容行业 tag)
--
-- industry_taxonomy: 行业树 (一级行业 + 二级子行业 + 自定义), 带关键词供规则分类器使用。
-- brand_intel_profiles: 每个 brand 的 Customer Intelligence Profile (Sincy-04 §9)。
-- intel_items.industries / tags: 每条情报的行业分类与标签 (Sincy-04 §7)。
-- publishing_packages.industry_tags: 内容按所属品牌行业打 tag, 支持各行各业客户。

CREATE TABLE IF NOT EXISTS industry_taxonomy (
    id          TEXT PRIMARY KEY,          -- slug, 如 technology / ai
    name        TEXT NOT NULL,             -- 展示名
    parent_id   TEXT REFERENCES industry_taxonomy(id) ON DELETE SET NULL,
    level       INTEGER NOT NULL DEFAULT 1,-- 1=一级行业 2=子行业 3=自定义
    keywords    JSONB NOT NULL DEFAULT '[]'::jsonb,  -- 分类关键词 (中英文)
    description TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS brand_intel_profiles (
    brand_id    UUID PRIMARY KEY REFERENCES brands(id) ON DELETE CASCADE,
    profile     JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE intel_items
    ADD COLUMN IF NOT EXISTS industries JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN IF NOT EXISTS tags       JSONB NOT NULL DEFAULT '{}'::jsonb;
CREATE INDEX IF NOT EXISTS idx_intel_items_industries ON intel_items USING gin (industries);

ALTER TABLE publishing_packages
    ADD COLUMN IF NOT EXISTS industry_tags JSONB NOT NULL DEFAULT '[]'::jsonb;

-- 行业树种子 (Sincy-04 §8, 12 个一级行业 + 子行业; 关键词中英文混合, 规则分类器用)
INSERT INTO industry_taxonomy (id, name, parent_id, level, keywords, description) VALUES
('technology','科技',NULL,1,'["technology","tech","ai","software","app","startup","程序","科技","软件","互联网","人工智能"]','Technology'),
('ai','人工智能','technology',2,'["ai","llm","gpt","agent","machine learning","aigc","生成式","智能体","大模型"]','AI / LLM / Agents'),
('saas','SaaS','technology',2,'["saas","subscription","b2b software","订阅软件"]','SaaS'),
('cybersecurity','网络安全','technology',2,'["cybersecurity","security","漏洞","黑客","安全"]','Cybersecurity'),
('cloud','云计算','technology',2,'["cloud","aws","azure","kubernetes","云服务","云原生"]','Cloud'),
('finance','金融',NULL,1,'["finance","bank","fintech","investment","stock","crypto","经济","金融","银行","股市","加密"]','Finance'),
('fintech','金融科技','finance',2,'["fintech","支付","数字银行","payment"]','Fintech'),
('investment','投资','finance',2,'["investment","vc","pe","fund","投资","融资","基金"]','Investment'),
('healthcare','医疗健康',NULL,1,'["healthcare","medical","hospital","pharma","医疗","医院","医药","健康","医疗器械"]','Healthcare'),
('pharma','医药','healthcare',2,'["pharma","drug","biotech","药物","生物医药","制药"]','Pharma'),
('wellness','大健康','healthcare',2,'["wellness","fitness","养生","健身"]','Wellness'),
('retail','零售电商',NULL,1,'["retail","ecommerce","e-commerce","shop","电商","零售","购物","品牌"]','Retail & E-commerce'),
('fashion','时尚','retail',2,'["fashion","apparel","服装","穿搭","潮牌"]','Fashion'),
('beauty','美妆','retail',2,'["beauty","skincare","makeup","cosmetics","美妆","护肤","彩妆"]','Beauty'),
('food','食品餐饮','retail',2,'["food","restaurant","catering","餐饮","食品","美食","餐厅"]','Food & Catering'),
('real_estate','房地产',NULL,1,'["real estate","property","housing","mortgage","房地产","房产","楼市","地产"]','Real Estate'),
('education','教育',NULL,1,'["education","edtech","learning","school","教育","培训","学校","课程"]','Education'),
('edtech','教育科技','education',2,'["edtech","online course","mooc","在线教育"]','EdTech'),
('manufacturing','制造业',NULL,1,'["manufacturing","factory","supply chain","制造","工厂","供应链"]','Manufacturing'),
('automotive','汽车',NULL,1,'["automotive","car","ev","tesla","汽车","新能源车","电动车"]','Automotive'),
('travel','旅游酒店',NULL,1,'["travel","hotel","tourism","旅游","酒店","出行"]','Travel & Hospitality'),
('media','媒体娱乐',NULL,1,'["media","entertainment","game","gaming","film","媒体","游戏","娱乐","影视"]','Media & Entertainment'),
('gaming','游戏','media',2,'["game","gaming","esports","手游","游戏"]','Gaming'),
('professional_services','专业服务',NULL,1,'["legal","consulting","accounting","law firm","律所","咨询","会计","法律"]','Professional Services'),
('marketing','营销广告',NULL,1,'["marketing","advertising","brand","seo","营销","广告","品牌营销"]','Marketing & Ads'),
('custom','自定义行业',NULL,3,'[]','用户自定义行业')
ON CONFLICT (id) DO NOTHING;
