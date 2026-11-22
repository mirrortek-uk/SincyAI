-- 0024: 可安装应用目录 (system.setup_install.<app> 的数据来源)。
-- 每行对应一个「签名 adapter + 白名单下载源」; 新增 app = 新增一行 + 重新签名入 app_adapter_versions。
CREATE TABLE IF NOT EXISTS setup_app_catalog (
    app_key             TEXT PRIMARY KEY,          -- e.g. 'x', 'wechat'
    display_name        TEXT NOT NULL,
    target_package      TEXT NOT NULL,             -- com.twitter.android
    download_url        TEXT NOT NULL,             -- 白名单直链
    sha256              TEXT NOT NULL,             -- APK 文件 SHA-256 (hex)
    signer_cert_sha256  TEXT NOT NULL DEFAULT '',   -- 安装后校验的签名证书指纹 (可空)
    active              BOOLEAN NOT NULL DEFAULT true,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 内置: X
INSERT INTO setup_app_catalog (app_key, display_name, target_package, download_url, sha256, signer_cert_sha256)
VALUES ('x', 'X (Twitter)', 'com.twitter.android',
        'https://REPLACE_WITH_WHITELISTED_X_APK_URL/x.apk',
        'REPLACE_WITH_APK_SHA256', '')
ON CONFLICT (app_key) DO NOTHING;
