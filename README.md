# Sincy — AI 内容运营平台

面向内容运营机构的多租户、自主化 AI 内容运营平台。Cloud AI Agency 为决策中枢，
Content Factory 生产多模态内容，经质量与合规门禁后封装为不可变 Publishing Package，
由 Execution Router 按"平台 × 动作"能力矩阵选择执行路径（官方 API → 设备界面自动化 → 辅助模式），
结果与平台数据回流策略层，形成"发现 → 策略 → 生产 → 检查 → 打包 → 路由执行 → 发布确认 → 分析学习"闭环。

设计文档（V1.1，修订设计稿）见上级目录 `../设计书/`：
- `Sincy-01_PRD_产品需求与业务规格书_V1.1.docx`（FR-001…FR-026）
- `Sincy-02_TDD_系统技术架构与详细设计书_V1.1.docx`（ADR-001…ADR-015）
- `Sincy-03_项目开发与实施指导书_V1.1.docx`（WP-01…WP-16）

## 产品愿景与当前状态

Sincy 不是"AI 生成一段文案"的工具，而是 **AI Digital Employee（数字员工）**：
用户只给一个运营目标（如"在 X 上建立 AI agents 领域的品牌权威")，数字员工自己完成
感知 → 理解 → 决策 → 计划 → 创作 → 审核 → 执行 → 衡量 → 学习 → 适应的完整周期。

当前已跑通的真实能力（非 mock)：

- **数字员工运营周期**: 6 个逻辑 Agent × 10 步周期，9 个 skill 域 / 77 个 skill 中 59 个已落地真 handler
  （research 直查情报库、content/creative 走 LLM、publishing 真派设备任务），每步证据落库可审计
- **世界情报 World Intelligence**: 26 个注册信源、定时采集入库（dedup 去重）、趋势聚合（velocity/growth/跨平台）、
  机会引擎打分、信源健康审计（healthy/degraded/failed/not_configured)
- **手机 Agent 真机执行**: 无障碍节点树 + Set-of-Mark 截图视觉融合，云端 LLM 逐步决策，
  人工化手势（贝塞尔滑动/抖动/压力变化）注入真实触摸；已在真实小米设备上验证通过 X 回帖全流程
- **全自动回帖流水线**: `POST /setup/reply_auto` 搜索 X → 打开最相关帖子 → 云端 LLM 按品牌语气生成人类口吻回复
  → 自动 approved → 签名 adapter 在详情页发出（回帖免人工审核）
- **安全与治理**: RLS 多租户隔离 + 负向测试、Argon2id、设备配对 challenge-response、Adapter HMAC 签名 +
  设备端验签、side-effect 后禁止自动重试、kill switch、预算熔断

快速上手（生产模式验证路径，本地 sincy_prod)：见下文「本地启动」；控制台 `http://localhost:3000`。

## 技术栈基线（TDD 3）

| 层 | 技术 |
| --- | --- |
| Cloud API | Rust 2024 (Edition 2024) + Axum + Tokio |
| Persistence | PostgreSQL + SQLx（迁移 + RLS 租户隔离） |
| Eventing | NATS JetStream + Transactional Outbox |
| Cache / Rate | Redis（仅快速限流，配额以 PG 账本为准） |
| Object Storage | S3-compatible |
| Frontend | Next.js + TypeScript |
| Android Agent | Kotlin（系统 API/无障碍）+ Rust Core（JNI：状态机与 Adapter 解释器） |
| Protocol | Protobuf over WSS/TLS，schema_version 显式协商 |
| Observability | OpenTelemetry / Prometheus / Grafana / 结构化日志 |

## 目录结构（TDD 5）

```
sincy/
  Cargo.toml            # workspace
  apps/                 # cloud-api, cloud-scheduler, agent-worker, router-worker, ...
  crates/               # sincy-common / sincy-config / sincy-protocol /
                        # sincy-database / sincy-security (+ 按边界新增)
  proto/                # 控制面 protobuf（WP-02）
  migrations/           # 有编号的数据库迁移（WP-03 起）
  adapters/             # 声明式 App Adapter + fixtures（与代码分离，签名发布）
  frontend/             # Next.js 控制台与客户审批入口
  android/              # Gradle/Kotlin Agent App
  docs/                 # 运行手册、ADR、验收脚本
```

## 本地启动（WP-01 验收路径）

需要 Rust 1.94.1+（`rust-toolchain.toml` 已锁定）与 Docker（本地 PG/NATS/Redis，见 `docs/dev-env.md`）。

```bash
cargo build --workspace
cargo test --workspace
cargo run -p cloud-api          # http://127.0.0.1:8080/api/v1/healthz
```

配置为分层加载：内置默认值 < `config/<env>.toml` < `SINCY_*` 环境变量。
密钥只允许以 `vault://` / `kms://` 引用形式存在，禁止提交真实凭证（见 `.gitignore`）。

环境变量：`SINCY_ENV`、`SINCY_HTTP_PORT`、`SINCY_DATABASE_URL`、`SINCY_NATS_URL`、
`SINCY_REDIS_URL`、`SINCY_LOG_FILTER`、`SINCY_LOG_JSON`、`SINCY_JWT_SECRET`、`SINCY_JWT_TTL_MINUTES`、
`SINCY_ALLOW_MOCK_ADAPTERS`（仅 local 可开，生产环境 config 校验拒绝）、`SINCY_MONTHLY_BUDGET_USD`、
`SINCY_LLM_PROVIDER`（`mock` | `openai-compatible`）、`SINCY_LLM_BASE_URL`、`SINCY_LLM_API_KEY`、
`SINCY_LLM_MODEL`、`SINCY_OBJECT_STORE_ROOT`、`SINCY_YOUTUBE_ACCESS_TOKEN`（本地 dev 兜底）。

## 认证与控制台（正式产品路径）

- 注册机构：`POST /api/v1/agencies {"name","owner_email","owner_password"}` → 机构 + owner 成员（Argon2id 哈希）+ JWT
- 登录：`POST /api/v1/auth/login {"email","password"}` → 短期 JWT（默认 60 分钟）
- 受保护路由一律校验 `Authorization: Bearer <jwt>`，**agency_id 只从令牌推导**，不信任客户端传参（防越权）
- 控制台：`frontend/` 下 `npm run dev` → http://localhost:3000 （未登录自动跳 /login；
  页面：总览 / 任务 / 设备 / AI 生成 / 急停开关 / 指标）
- 生产环境发布走**真实平台 API adapter**：
  - YouTube: Data API v3 `videos.insert` resumable upload，配额 1600/call，媒体从对象存储取
  - **X (Twitter)**: 不走付费 API v2 —— 走**设备自动化路线**（PRD 5.3 "按成本评估是否走设备"
    + HID 设备交互设计书：平台 API 付费/不可用时由设备层执行，不绕过平台安全机制）。
    能力矩阵（migration 0014）：`device` 路线 verified 优先级 10（签名 adapter
    `adapters/x.publish_post.yaml`，目标 `com.twitter.android`，挑战界面→WAITING_USER），
    `api` 路线降级为 unverified 优先级 50（标注 `paid_tier_required`，仅作付费兜底），
    `assisted` 90 兜底；任务以 QUEUED 进入 device_tasks 由 gateway 通过 WSS 分发给在线 agent。
    X API v2 adapter（`sincy-platform::x`）保留为付费 tier 回退，不删除。
  - mock adapter 仅在 `SINCY_ALLOW_MOCK_ADAPTERS=true`（local）可用，
    `sincy-config` 在非 local 环境拒绝该配置。账号 OAuth token 只存 `vault://` 引用。
- AI 生成：`POST /api/v1/ai/generate` 受月预算熔断（FR-026）保护，成本落账 `model_cost_records`
- 指标导入：`POST /api/v1/analytics/metrics`（source 必须标注 api|device|manual）

## 内容从哪里来（X/YouTube 发布内容的来源）

内容 = **内容包（publishing_packages）**，来源两条路：
1. **AI 生成页** `/ai`：`POST /ai/generate` 带 `save_as_package` → 生成文本自动存包
   （带 provenance，状态 `pending`，须人工 `POST /packages/{id}/approve` 后才可发布，FR-024）；
2. **手工创建**：`POST /packages`（状态直接 `approved`）。
   X 文案在入库时即校验 280 字上限。

任务页 `/tasks` 的品牌/内容包都是下拉选择（`GET /brands`、`GET /packages?platform=&action=`），
不再手填 UUID；发布时后端强制校验包已审批（否则 409 `package_not_approved`）。

## 设备 Agent（手机端，X 等平台走设备路线时使用）

- 代码：`android/`（Kotlin App `com.sincy.agent` + `android/rust-core` Rust cdylib）。
  真正的执行逻辑在 `crates/sincy-agent`（host 可测），Android 侧只是 JNI 薄包装，
  保证手机端行为与服务端策略零偏差。
- 配对：App 内登录账号 → `POST /devices/register/start`（Bearer）→ 拿到一次性
  device_secret + nonce → proof = sha256(sha256(secret)+nonce) →
  `POST /devices/register/complete`（公开，proof 即凭证）→ 设备 token 存
  EncryptedSharedPreferences（Keystore）。服务端只存密钥哈希。
- 运行：前台服务连 WSS `/api/v1/agent/connect?token=...`，心跳 10s；
  云端把 QUEUED 任务以 `device.task.assign` 派发；agent 用
  `GET /agent/tasks/{id}`（Bearer 设备 token）拉任务包 + 签名 adapter，
  验签（HMAC，pinned key）+ 版本门禁（version_range）后由解释器逐步执行，
  每步证据上报；挑战界面 → WAITING_USER 转人工；side_effect 之后失败一律
  VERIFICATION_PENDING，绝不自动重试。
- 本地验证（不用手机）：`cargo run -p sincy-agent --bin sincy-agent-sim -- ...`
  用脚本化屏幕跑真实 agent 核心走完整 WSS 链路；集成测试
  `apps/cloud-api/tests/device_agent_e2e.rs` 断言任务从 QUEUED 到 COMPLETED。
- 打包 APK：需要 Android SDK platform 35 + build-tools + cargo-ndk
  （本机 SDK 为空，CI 或装了 SDK 的机器上 `./gradlew assembleRelease`）。

## 工程规则（指导书 3.2）

- 一次只实现一个可验收的垂直切片；先读 PRD/TDD 与现有代码，不凭空假设文件/表/API 存在。
- 请求与任务处理路径禁止无理由的 `unwrap`/`expect`。
- 错误分五类：可重试 / 不可重试 / 需用户操作 / 无副作用失败（可降级）/ 结果不确定（不可降级）。
- 所有外部副作用先定义幂等策略、核验方式与 side_effect 边界；越过 side_effect 步骤后不得自动重试或降级。
- 数据库变更只走有编号 migration；运行时代码不得改 schema。
- App Adapter 是数据不是代码：声明式文件 + schema 校验 + 签名后发布，设备端验签。
- 不实现验证码处理、设备指纹伪造、多账号防关联等规避平台安全机制的功能；挑战一律转人工（WAITING_USER）。

## CI 质量门禁（指导书 8 / DoD）

`cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、
`cargo test --workspace`、Adapter schema 校验。迁移需在空库与升级库均可运行。

## 实施进度（WP-01 … WP-16，对照指导书 §5）

| WP | 内容 | 状态 |
| --- | --- | --- |
| WP-01 | 仓库与 CI 基础（workspace、fmt/clippy/test 门禁） | ✅ 已完成 |
| WP-02 | Protocol（Envelope + 18 种消息类型、golden fixtures、protox codegen） | ✅ 已完成 |
| WP-03 | 数据库基础与租户隔离（11 个 migration、RLS + 复合租户键、RLS 负向测试） | ✅ 已完成 |
| WP-04 | 内容生产/审批/发布包（不可变 PublishingPackage、QualityGate、AI 标识） | ✅ 已完成 |
| WP-05/06 | Execution Router、能力矩阵、配额账本、Mock YouTube Adapter | ✅ 已完成 |
| WP-07/08 | Device Registry（challenge-response、JWT 会话）+ WSS 网关 | ✅ 已完成 |
| WP-09 | 任务编排状态机（守卫迁移、幂等、kill switch） | ✅ 已完成 |
| WP-10 | Transfer Service（8 MiB 分块、双重 sha256、断点续传） | ✅ 已完成 |
| WP-11/12 | Android Agent：无障碍感知 + 录屏视觉(SoM) + agentic LLM 闭环 + 人工化手势，真机已验证 X 回帖 | ✅ 已完成（真机验证通过） |
| WP-13 | App Adapter 框架与 Drift 检测（schema、签名验签、version_range、降级）+ agentic adapter(system.agentic) | ✅ 已完成 |
| WP-14 | 分析回流 + 急停 + 预算熔断（sincy-ai：BudgetChecker、LlmGateway、Analytics） | ✅ 已完成 |
| WP-15 | 可观测性与运维（/metrics、日志脱敏、重入队恢复、Runbook、备份/PITR 脚本） | ✅ 已完成 |
| WP-16 | Windows Agent | ⏳ MVP 后 |
| — | 前端 Agency Console（Next.js：总览/发布/急停/指标） | ✅ 骨架已完成（`frontend/`） |

质量门禁当前状态：`cargo fmt` / `cargo clippy --workspace --all-targets -- -D warnings` / 
`SINCY_TEST_DATABASE_URL=... cargo test --workspace` 全绿（62 个测试套件）。
集成测试依赖本地 PostgreSQL：`SINCY_TEST_DATABASE_URL="postgres://postgres:password@127.0.0.1:5432/sincy_test"`。

ADR 落地口径见 `docs/ADR.md`；故障恢复见 `docs/runbook.md`；备份/PITR 见 `docs/backup.md`。
