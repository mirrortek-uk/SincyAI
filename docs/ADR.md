# Sincy 架构决策记录（ADR-001 .. ADR-015）

来源：TDD V1.1 §17 ADR 清单。状态为实施至今的落地口径；"待决策"项保持开放。

| ADR | 决策 | 状态 | 落地（本仓库） |
|---|---|---|---|
| ADR-001 | Rust-first Cloud 与 Agent Core | 已选基线 | `crates/*` + `apps/*` 全 Rust；Agent Core 以 Rust JNI 形态嵌入 Android |
| ADR-002 | Kotlin + Rust JNI Android 分层 | 已选基线 | Android app 骨架（WP-11/12，设备矩阵验证待真机） |
| ADR-003 | Windows Service + Desktop Helper；MVP 可选 | 修订 | WP-16，MVP 后推进；`sincy-transfer` 已预留本地传输抽象 |
| ADR-004 | Cloud 为权威控制面；设备直连 Cloud | 已选基线 | cloud-api WSS 网关 `/api/v1/agent/connect`，设备 JWT 会话直连 |
| ADR-005 | WSS 控制面、大文件独立传输 | 已选基线 | sincy-protocol Envelope over WSS；sincy-transfer 分块续传（8 MiB 分块 + 双重 sha256） |
| ADR-006 | Publishing Runtime 与通用 Device Automation 分离 | 已选基线 | sincy-platform (API Adapter) vs sincy-adapter/sincy-device (设备自动化) |
| ADR-007 | NATS JetStream 与 Transactional Outbox | 建议，实施验证 | Outbox/事件表已落（task_events 追加写）；NATS 接入在 WP-09 编排扩展，当前同步 dispatch + 2s 轮询兜底 |
| ADR-008 | PostgreSQL RLS 与复合租户约束 | 必须设计并测试 | 全业务表 `FORCE ROW LEVEL SECURITY` + `(agency_id, id)` 复合唯一键；`tests/rls_isolation.rs` 覆盖跨租户读写拒绝与无上下文 fail-closed |
| ADR-009 | 首发平台：YouTube（API）+ 一个设备渠道 | 修订，设备渠道待选 | MockYouTubeAdapter（显式标注 mock，生产环境拒绝）；设备渠道首发小红书 Adapter 样例 `adapters/xiaohongshu.publish_video.yaml` |
| ADR-010 | 部署区域、KMS、对象存储供应商 | 待决策 | sincy-config 分层配置预留；对象存储抽象 `ObjectStore`（LocalObjectStore 已实现，S3 后续） |
| ADR-011 | 混合执行：能力矩阵驱动的 Execution Router 与降级链 | 新增，已选 | sincy-routing：优先级 + 覆盖 + 前置条件 + skipped-reasons 审计；NoSideEffect→降级、Uncertain→VERIFICATION_PENDING |
| ADR-012 | 单一 Platform App + 共享配额账本 | 新增，已选 | quota_pools + quota_ledger，预留/确认/释放同事务 FOR UPDATE，按 (agency, task) 幂等 |
| ADR-013 | 屏幕感知控件树优先，OCR/视觉兜底 | 新增，已选 | sincy-adapter 解释器基于锚点（控件树）匹配；OCR/视觉层 WP-11 待真机 |
| ADR-014 | 声明式、签名、版本绑定的 App Adapter | 新增，已选 | YAML schema + HMAC 签名 + VersionRange（语义化范围）；版本越界拒绝执行并报 ui_drift |
| ADR-015 | 视觉模型端侧 vs 云端 | 新增，待决策 | sincy-ai 的 LlmGateway 抽象预留云端视觉；端侧待设备算力评估后决策 |

## 补充实施决策（实现过程中新增）

- **错误五分类**（Guide 3.2）：Retryable / NonRetryable / NeedsUser / NoSideEffect / Uncertain，全链路统一 JSON 错误体（sincy-common）。
- **幂等**：发布接口 `(agency_id, idempotency_key)` 回放；`task.result` 仅在执行态生效；传输分块按 sha256 幂等。
- **副作用边界**：`side_effect_done=true` 后设备断线一律 VERIFICATION_PENDING，绝不自动重试/降级（tests/resilience.rs）。
- **Mock紀律**：Mock Adapter / Mock LLM 一律显式标注，生产路径拒绝（DoD 红线）。
- **观测**：/metrics Prometheus 计数器 + SECURITY DEFINER 仪表函数；日志统一 redact 脱敏。
