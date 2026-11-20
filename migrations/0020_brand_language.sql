-- 0020_brand_language: 品牌/员工输出语言设置 (默认中文 zh-CN)
-- 品牌级语言是该品牌下所有数字员工内容的默认输出语言;
-- 员工可在 identity.language 上覆盖 (employee_identity_versions.identity JSONB)。

BEGIN;

ALTER TABLE brands
    ADD COLUMN IF NOT EXISTS language TEXT NOT NULL DEFAULT 'zh-CN'
    CHECK (language IN ('zh-CN','zh-TW','en-US','ja-JP','ko-KR','fr-FR','de-DE','es-ES','pt-BR'));

COMMIT;
