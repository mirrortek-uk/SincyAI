-- 0023_device_pairing_codes: 配对码 (code-only pairing).
-- 控制台(已登录 owner)生成一次性配对码; 手机 agent 只凭码完成配对,
-- 不需要在设备上输入账号密码。码为 8 位大写字母+数字(去掉 0/O/1/I),
-- 10 分钟有效、一次性使用。

BEGIN;

CREATE TABLE IF NOT EXISTS device_pairing_codes (
    code        TEXT PRIMARY KEY,
    agency_id   UUID NOT NULL REFERENCES agencies (id),
    created_by  UUID NOT NULL,               -- agency_members.id
    expires_at  TIMESTAMPTZ NOT NULL,
    used_at     TIMESTAMPTZ,
    device_id   UUID                         -- 配对成功后回填
);
CREATE INDEX IF NOT EXISTS idx_pairing_codes_live
    ON device_pairing_codes (expires_at) WHERE used_at IS NULL;

COMMIT;
