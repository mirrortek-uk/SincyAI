-- 0027: adapter 版本记录签名方案 (hmac-sha256 默认, ed25519 为生产方案: 设备只持公钥)
ALTER TABLE app_adapter_versions ADD COLUMN IF NOT EXISTS signature_scheme TEXT NOT NULL DEFAULT 'hmac-sha256';
