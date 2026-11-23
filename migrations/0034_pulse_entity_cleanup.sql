-- 0034_pulse_entity_cleanup: 清理 WebScrapeProvider 早期版本抓进来的页面 chrome 垃圾实体
-- (导航链接 / 语言名 / 低频话题标签等), 以及它们镜像进 intel_items 的条目。
-- 判定: canonical_key 含 ':tag:' 且标题不在高频白名单, 或标题命中导航黑名单。

DELETE FROM pulse_observations
 WHERE entity_id IN (
    SELECT e.id FROM pulse_entities e
     WHERE e.canonical_key LIKE '%:tag:%'
        OR e.title ~* '^(blog|faq|privacy|follow us|see the full|leaderboard|中文|日本語|한국어|english|español|français|português|ภาษาไทย|türkçe|statistics|methodology|monitoring|help|about|contact|terms)$'
        OR length(e.title) < 4
 );

DELETE FROM pulse_entities
 WHERE canonical_key LIKE '%:tag:%'
    OR title ~* '^(blog|faq|privacy|follow us|see the full|leaderboard|中文|日本語|한국어|english|español|français|português|ภาษาไทย|türkçe|statistics|methodology|monitoring|help|about|contact|terms)$'
    OR length(title) < 4;

DELETE FROM intel_items
 WHERE dedup_key LIKE 'pulse:%'
   AND (payload->>'title') ~* '^(blog|faq|privacy|follow us|see the full|leaderboard|中文|日本語|한국어|english|español|français|português|ภาษาไทย|türkçe|statistics|methodology|monitoring|help|about|contact|terms)$'
   OR (dedup_key LIKE 'pulse:%' AND length(payload->>'title') < 4);

-- 十六进制颜色 / 纯数字伪话题 (如 #0055DC, #39) 一并清理
DELETE FROM pulse_observations
 WHERE entity_id IN (
    SELECT e.id FROM pulse_entities e
     WHERE e.canonical_key ~ ':tag:[0-9a-fA-F]{3,8}$' OR e.title ~ '^#[0-9]'
 );
DELETE FROM pulse_entities
 WHERE canonical_key ~ ':tag:[0-9a-fA-F]{3,8}$' OR title ~ '^#[0-9]';
DELETE FROM intel_items
 WHERE dedup_key LIKE 'pulse:%' AND (payload->>'title') ~ '^#[0-9]';

-- 早期版本把页面里所有 #xxx (含 CSS 颜色) 都抓成了 tag 实体, 统一清掉, 由新版解析器重抓
DELETE FROM pulse_observations WHERE entity_id IN (SELECT id FROM pulse_entities WHERE canonical_key LIKE '%:tag:%');
DELETE FROM pulse_entities WHERE canonical_key LIKE '%:tag:%';
DELETE FROM intel_items WHERE dedup_key LIKE 'pulse:%' AND (payload->>'title') LIKE '#%';
