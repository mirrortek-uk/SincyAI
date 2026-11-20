# Sincy Runbook (WP-15)

可观测性与故障恢复手册。所有故障场景的验收标准：可定位（指标/日志）、可恢复（操作步骤）、恢复后无重复外部副作用。

## 1. 观测面

| 面 | 位置 | 内容 |
|---|---|---|
| 指标 | `GET /api/v1/metrics` (Prometheus text) | 计数器：请求、发布、路由分流、配额、急停、Agent 连接/断线/任务下发/结果/重入队、UI Drift、挑战界面、预算熔断；仪表：任务分状态、在线设备、降级 Adapter、近 1h Drift |
| 日志 | stdout, `SINCY_LOG_FILTER` 控制 (默认 `info`), `SINCY_LOG_JSON=true` 输出 JSON | 结构化 tracing，每请求 `request_id`（`x-request-id` 回头）；敏感字段经 `sincy_common::redact` 脱敏 |
| 事件 | `task_events` / `audit_events` / `ui_drift_events` (追加写) | 状态迁移与审计投影 |

## 2. 告警阈值（建议）

| 告警 | 条件 | 级别 |
|---|---|---|
| `sincy_agent_tasks_requeued_total` 持续增长 | 设备频繁掉线 | P2 |
| `sincy_ui_drift_last_hour > 0` | Adapter 降级 | P2 |
| `sincy_adapters_degraded > 0` | 需要发布新 Adapter 版本 | P2 |
| `sincy_quota_exceeded_total` 增长 | 配额耗尽，检查套餐/扩容 | P3 |
| `sincy_kill_switch_active = 1` | 急停生效，需人工确认恢复 | P1 |
| `sincy_devices_online = 0` 且有 QUEUED 任务 | 所有设备离线 | P1 |

## 3. 故障场景与恢复

### F1 设备离线 / 断网
- 定位：`sincy_devices_online` 下降、`sincy_agent_disconnects_total` 增长、日志 `agent disconnected`。
- 行为：断线时云端自动把该设备 `EXECUTING` 且 `side_effect_done=false` 的任务重入队 `QUEUED`（`sincy_agent_tasks_requeued_total`）；设备恢复后自动重派。
- 恢复：无需人工。若设备凭证被撤销（F3），重连会被拒绝。

### F2 Worker 崩溃 / 重复消息
- 定位：`task_events` 中同一 `idempotency_key` 的重复 `task.result` 被忽略（日志 `duplicate task.result ignored`）。
- 行为：发布接口按 `(agency_id, idempotency_key)` 幂等回放；`task.result` 仅在 `EXECUTING/EXECUTING_STEPS` 状态生效，重复投递不产生第二个事件。
- 恢复：无需人工。`tests/resilience.rs` 覆盖。

### F3 凭证撤销
- 定位：连接时 `device_auth_failed`（鉴权在升级 WebSocket 之前完成）。
- 行为：被撤销设备无法再连接；进行中的 socket 在心跳/下次读写时断开，任务按 F1 重入队。
- 恢复：设备重新走 challenge-response 注册（secret 只返回一次，库内只存哈希）。

### F4 配额耗尽
- 定位：`sincy_quota_exceeded_total` 增长，发布返回 `QUOTA_WAIT` + `Degrade(Assisted)`。
- 行为：预留、确认、释放在同一事务内更新池余额（FOR UPDATE），不会超发；释放后余额恢复。
- 恢复：扩容 `quota_pools.limit_value` 或升级套餐；无需重启。

### F5 UI Drift
- 定位：`sincy_ui_drift_events_total` / `sincy_ui_drift_last_hour`、`sincy_adapters_degraded`。
- 行为：锚点匹配率低于阈值或 App 版本越界时，解释器零副作用停止，上报 `ui_drift.detected`，云端把对应 `channel_capabilities` 标记 `degraded`，路由自动降级到下一优先级（API 或 Assisted）。
- 恢复：发布新 Adapter 版本（签名 + version_range），在测试机矩阵跑 Adapter 回归后灰度上线。

### F6 挑战界面（验证码/安全验证）
- 定位：`sincy_challenges_total` 增长，任务进入 `WAITING_USER`。
- 行为：**永远不自动解决**。任务停泊等待人工在设备上处理，处理后由人工触发恢复（`WAITING_USER → EXECUTING_STEPS`）。
- 恢复：人工完成挑战后在控制台恢复任务。

### F7 副作用后中断（最危险）
- 定位：`side_effect_done=true` 且设备断线。
- 行为：任务被置为 `VERIFICATION_PENDING`（Uncertain），**绝不自动重试、绝不降级**，等待人工核验外部平台是否已发布。
- 恢复：人工确认后手动置为 `PUBLISHED` 或 `FAILED`。

### F8 预算熔断（FR-026）
- 定位：`sincy_budget_blocked_total` 增长，生成任务返回 `budget exceeded`。
- 行为：`BudgetChecker::check` 在生成前按 agency/品牌月预算拦截；实际成本落账 `model_cost_records`。
- 恢复：调高月预算或等待次月重置。

## 4. 日志脱敏

`sincy_common::redact` 会对 `token/secret/password/authorization/api_key/session/proof/nonce` 等键值和长度 ≥16 的凭证形态串（JWT、hex、base64）打码为 `***`。新增日志字段前请确认不含明文密钥；宁可误杀不可漏脱。

## 5. 备份与恢复

见 [backup.md](backup.md)。脚本：`scripts/backup.sh`（全量+ WAL 归档）、`scripts/restore_drill.sh`（恢复演练）。
