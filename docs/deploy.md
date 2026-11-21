# 部署清单：Vercel (frontend + API) + Supabase (Postgres)

当前所有配置都是环境变量驱动（`sincy-config`），云端迁移不需要改代码。

## Supabase

1. 新建项目 → Settings → Database → Connection string（用 **Session pooler**，
   端口 5432，`postgres://postgres.<ref>:[PASSWORD]@aws-0-...:5432/postgres`）。
2. `SINCY_DATABASE_URL` 指到 pooler。迁移由服务启动时 `sqlx::migrate!` 自动应用
   （migrations 0001–0015，幂等）。
3. 建一个非超级用户应用角色（生产强制 RLS 生效）：
   ```sql
   CREATE ROLE sincy_app LOGIN PASSWORD '...';
   GRANT USAGE ON SCHEMA public TO sincy_app;
   GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO sincy_app;
   GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO sincy_app;
   ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO sincy_app;
   ```
   （表上已有 `ENABLE/FORCE ROW LEVEL SECURITY` + `app.current_agency_id` 策略，
   超级用户会绕过 RLS，生产必须用 sincy_app。）
4. Storage 桶（对象存储）：`SINCY_OBJECT_STORE_ROOT` 本地是目录；生产可换 S3 兼容
   （ADR-010 待定），媒体走 transfer 服务，绝不进 WSS 信封。

## Vercel（cloud-api）

cloud-api 是 axum 二进制；Vercel 需要 serverless 形态 —— 两条路：
- 推荐：API 部署到任意支持长连接的平台（Fly.io/Render/Railway）跑 `cloud-api`，
  因为 agent WSS 长连接在 Vercel serverless 上会被断开。
- 前端（`frontend/`，Next.js）放 Vercel：`SINCY_API_INTERNAL` 指到上面的 API 地址，
  `next.config.mjs` 的 rewrites 把 `/api/v1/*` 代理过去；
  `SINCY_CORS_ORIGINS` 加上 Vercel 域名。

环境变量（Vercel/平台 secrets）：

| 变量 | 说明 |
|---|---|
| `SINCY_ENV` | `production`（禁 mock adapter、禁 dev JWT secret） |
| `SINCY_DATABASE_URL` | Supabase pooler 连接串 |
| `SINCY_JWT_SECRET` | 强随机，至少 32 字节 |
| `SINCY_ADAPTER_SIGNING_KEY` | 强随机；并用 `cargo run -p sincy-agent --example sign_adapter` 重签 adapter 重新入库 app_adapter_versions（migration 0015 里是 dev key，生产必须替换） |
| `SINCY_CORS_ORIGINS` | `https://<vercel-app>.vercel.app` |
| `SINCY_HTTP_PORT` | 平台注入的 PORT |
| `SINCY_YOUTUBE_ACCESS_TOKEN` / `vault://` 引用 | 平台 OAuth token 只存引用 |
| `SINCY_LLM_MODEL` + API key | AI 生成（受月预算熔断） |

## 上线前检查

- [ ] 生产库用非超级用户连接（RLS 真正生效），跑一次 `crates/sincy-database/tests/rls_isolation.rs`
- [ ] `SINCY_ENV=production` 启动成功（config 校验拒绝 mock/dev secret）
- [ ] 换 adapter signing key 并重新 seed `app_adapter_versions`
- [ ] 设备配对全链路在真机走通（注册 → WSS → 派发 → 执行 → COMPLETED）
- [ ] X 走 device 路线（capability 矩阵 device=verified@10），付费 API 仅作兜底

## 首管理员引导（生产环境没有内置 demo 账号）

服务启动时若设置以下环境变量，会**幂等地**创建第一个 agency + owner（密码只存 Argon2id 哈希）：

| 变量 | 说明 |
|---|---|
| `SINCY_BOOTSTRAP_ADMIN_EMAIL` | 首管理员邮箱 |
| `SINCY_BOOTSTRAP_ADMIN_PASSWORD` | 强密码，进程环境传入，不落库明文 |
| `SINCY_BOOTSTRAP_AGENCY_NAME` | 可选，默认 `Sincy` |

`SINCY_ENV != local` 时强制生效：`SINCY_ALLOW_MOCK_ADAPTERS` 被忽略、`SINCY_JWT_SECRET` 缺失直接启动失败、LLM 的 `mock` / `manual-file` provider 被拒绝、`openai-compatible` 必须带 `SINCY_LLM_API_KEY`。`GET /api/v1/healthz` 会返回 `env / allow_mock_adapters / llm_provider / database`，一眼可辨是不是真环境。

## 本地生产库验证（不用 demo 库）

本机可起独立的生产模式实例做上线前验证（与 demo 完全隔离）：

```bash
createdb sincy_prod
SINCY_ENV=production \
SINCY_DATABASE_URL=postgres://...@127.0.0.1:5432/sincy_prod \
SINCY_HTTP_PORT=18081 \
SINCY_JWT_SECRET=$(openssl rand -hex 32) \
SINCY_BOOTSTRAP_ADMIN_EMAIL=admin@yourco.com \
SINCY_BOOTSTRAP_ADMIN_PASSWORD=$(openssl rand -urlsafe 24) \
SINCY_LLM_PROVIDER=openai-compatible SINCY_LLM_API_KEY=... \
cargo run -p cloud-api
```

## 前端上 Vercel

`frontend/` 已带 `vercel.json`；Vercel 项目里设置 `SINCY_API_INTERNAL=https://<cloud-api 地址>`，`next.config.mjs` 的 rewrites 会把 `/api/v1/*` 代理过去（同域部署，浏览器零 CORS 问题）。注意 agent 的 WSS 长连接不能走 Vercel serverless，cloud-api 请部署在支持长连接的平台（见上文）。
