-- 0015_seed_x_adapter: register the signed X device adapter in the platform
-- adapter registry so agents can fetch + verify it (GET /api/v1/agent/tasks/{id}).
-- Signature = HMAC-SHA256 over the canonical JSON definition with the platform
-- signing key. Local/dev key: 'sincy-dev-adapter-signing-key' (SINCY_ADAPTER_SIGNING_KEY);
-- production deployments MUST re-seed with a strong key via ops tooling
-- (cargo run -p sincy-agent --example sign_adapter -- adapters/x.publish_post.yaml <key>).

BEGIN;

INSERT INTO app_adapters (id, app_package, description)
VALUES ('x.publish_post', 'com.twitter.android', 'X compose flow via device automation (no paid API; PRD 5.3 / HID doc)')
ON CONFLICT (id) DO NOTHING;

INSERT INTO app_adapter_versions (adapter_id, version, version_range, definition, signature, sha256, state)
VALUES (
    'x.publish_post',
    1,
    '>=10.0 <11.0',
    '{"adapter":"x.publish_post","version":1,"app":{"package":"com.twitter.android","version_range":">=10.0 <11.0"},"screens":{"challenge":{"anchors":[{"id":null,"text":null,"text_any":["Verify you are human","unusual activity","安全验证","人机验证"]}]},"composer":{"anchors":[{"id":"com.twitter.android:id/tweet_text","text":null,"text_any":null},{"id":null,"text":"What''s happening?","text_any":null}]},"home":{"anchors":[{"id":"com.twitter.android:id/bottom_bar","text":null,"text_any":null},{"id":null,"text":"Home","text_any":null}]},"posted_confirm":{"anchors":[{"id":null,"text":null,"text_any":["Your post was sent","Post sent","已发布"]}]},"update_dialog":{"anchors":[{"id":null,"text":null,"text_any":["Update now","立即更新"]}]}},"flow":[{"expect":"home","action":"tap","target":{"id":"com.twitter.android:id/composer_fab","text":null,"text_any":null},"value":null,"side_effect":false,"verify":false},{"expect":"composer","action":"set_text","target":{"id":"com.twitter.android:id/tweet_text","text":null,"text_any":null},"value":"package.text","side_effect":false,"verify":false},{"expect":"composer","action":"tap","target":{"id":null,"text":"Post","text_any":null},"value":null,"side_effect":true,"verify":false},{"expect":"posted_confirm","action":"back","target":null,"value":null,"side_effect":false,"verify":true}],"known_interrupts":[{"screen":"update_dialog","action":"tap","target":{"id":null,"text":null,"text_any":["Not now","以后再说"]}}]}'::jsonb,
    '8XtVn729kYqAWSrgiUfaIHZlE2LsZLY8HddK1f/OYqM=',
    'de14dbf8317a139eb60e500ea54a9bc52a7ebf7ac3aa0534eded7a0c66377d1a',
    'active'
)
ON CONFLICT (adapter_id, version) DO UPDATE SET
    definition = EXCLUDED.definition,
    signature = EXCLUDED.signature,
    sha256 = EXCLUDED.sha256,
    state = 'active';

COMMIT;
