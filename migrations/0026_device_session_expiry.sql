-- 0026: device_sessions 增加过期时间 + 按 token_hash 精确校验会话 (撤销/轮换可真正踢下线)
ALTER TABLE device_sessions ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ;
CREATE INDEX IF NOT EXISTS device_sessions_device_state_idx ON device_sessions (device_id, state);
