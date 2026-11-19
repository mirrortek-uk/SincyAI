# Sincy — AI 内容运营平台

面向内容运营机构的多租户、自主化 AI 内容运营平台。Cloud AI Agency 为决策中枢，
Content Factory 生产多模态内容，经质量与合规门禁后封装为不可变 Publishing Package，
由 Execution Router 按"平台 × 动作"能力矩阵选择执行路径（官方 API → 设备界面自动化 → 辅助模式），
结果与平台数据回流策略层，形成"发现 → 策略 → 生产 → 检查 → 打包 → 路由执行 → 发布确认 → 分析学习"闭环。

设计文档（V1.1，修订设计稿）见上级目录 `../设计书/`：
- `Sincy-01_PRD_产品需求与业务规格书_V1.1.docx`（FR-001…FR-026）
- `Sincy-02_TDD_系统技术架构与详细设计书_V1.1.docx`（ADR-001…ADR-015）
- `Sincy-03_项目开发与实施指导书_V1.1.docx`（WP-01…WP-16）

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
`SINCY_REDIS_URL`、`SINCY_LOG_FILTER`、`SINCY_LOG_JSON`。

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
