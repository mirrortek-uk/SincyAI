-- 0045: 签名并启用 x.read_mentions 只读提及抓取适配器 (互动子循环入口; dev 签名密钥, prod 由 ops 重新播种)
INSERT INTO app_adapters (id, app_package, description)
VALUES ('x.read_mentions', 'com.twitter.android', 'X mentions read-only scrape via device automation (pure perception; challenge => WAITING_USER)')
ON CONFLICT (id) DO NOTHING;

INSERT INTO app_adapter_versions (adapter_id, version, version_range, definition, signature, sha256, state)
VALUES (
    'x.read_mentions',
    1,
    '>=10.0',
    '{"adapter":"x.read_mentions","version":1,"app":{"package":"com.twitter.android","version_range":">=10.0"},"screens":{"challenge":{"anchors":[{"id":null,"text":null,"text_any":["人机验证","Verify you are human","unusual activity","安全验证"],"near_text_any":null}]},"main_nav":{"anchors":[{"id":null,"text":null,"text_any":["主页","Home"],"near_text_any":null},{"id":null,"text":null,"text_any":["通知标签","Notifications","私信","Messages"],"near_text_any":null}]},"notifications":{"anchors":[{"id":null,"text":null,"text_any":["通知","Notifications"],"near_text_any":null},{"id":null,"text":null,"text_any":["提及","Mentions","回复","Replies"],"near_text_any":null}]}},"flow":[{"expect":"main_nav","action":"tap","target":{"id":null,"text":null,"text_any":["通知标签","Notifications"],"near_text_any":null},"value":null,"side_effect":false,"capture":null,"verify":false},{"expect":"notifications","action":"wait","target":null,"value":"2500","side_effect":false,"capture":null,"verify":false},{"expect":"notifications","action":"tap","target":{"id":null,"text":null,"text_any":["提及","Mentions"],"near_text_any":null},"value":null,"side_effect":false,"capture":null,"verify":false},{"expect":"notifications","action":"wait","target":null,"value":"2000","side_effect":false,"capture":null,"verify":false},{"expect":"notifications","action":"read","target":{"id":null,"text":null,"text_any":null,"near_text_any":["提及","Mentions"]},"value":null,"side_effect":false,"capture":"mentions","verify":false}],"known_interrupts":[],"params":null}'::jsonb,
    'zav/PqsI0kkhRZgwpneX0/zxT8MWJ+o202kwcnxbiOQ=',
    '5aa032295d789ad0f3c8de9ba4252e184b25206a0d11120cdbf4665f5040426a',
    'active'
)
ON CONFLICT (adapter_id, version) DO UPDATE SET
    definition = EXCLUDED.definition,
    signature = EXCLUDED.signature,
    sha256 = EXCLUDED.sha256,
    state = EXCLUDED.state;
