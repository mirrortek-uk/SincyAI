-- 0032_pulse_x_providers_public: 7 个 X 生态情报站确认为公开数据, 允许抓取。
-- legal_status unknown -> public, enabled=true, manifest 内 legal_status 同步,
-- 由 60s 调度器按各 dataset 的 update_frequency 轮询采集入库。

UPDATE pulse_providers
   SET legal_status = 'public',
       enabled = true,
       manifest = jsonb_set(manifest, '{legal_status}', '"public"'::jsonb, true)
 WHERE id IN ('xbangdan','xpaiming','shadowban-radar','getxanalytics','xranks','trends24','tide-now');
