//! GET /api/v1/architecture — 结构化系统架构模型 (as-built), 供后台
//! console 的「架构」页渲染成 SVG 分层架构图 + 数据流。纯静态描述,
//! 不访问数据库; 节点/边/流都在本文件维护, 与代码结构同步更新。

use axum::{Json, Router, extract::State, routing::get};
use serde_json::{Value, json};
use sqlx::Row;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/architecture", get(architecture))
}

fn node(id: &str, title: &str, tech: &str, desc: &str) -> Value {
    json!({ "id": id, "title": title, "tech": tech, "desc": desc })
}

fn edge(from: &str, to: &str, label: &str, kind: &str) -> Value {
    json!({ "from": from, "to": to, "label": label, "kind": kind })
}

/// 6 个核心 Agent (DE guide V1.1 §"Six initial agents") — 稳定 id 供架构图/动画渲染。
fn agents_model() -> Value {
    json!([
        { "id": "executive", "name": "Executive Agent", "name_zh": "执行主管",
          "role": "理解目标、协调其他 Agent、任务决策、最终结果",
          "steps": ["observe", "decide", "execute", "adapt"], "domains": [] },
        { "id": "research_strategy", "name": "Research & Strategy Agent", "name_zh": "研究与战略",
          "role": "市场研究、趋势分析、受众分析、内容战略",
          "steps": ["understand", "plan"], "domains": ["research", "strategy"] },
        { "id": "creative", "name": "Creative Agent", "name_zh": "创意 Agent",
          "role": "内容创意、选题、角度、Hook、创意方向",
          "steps": ["plan"], "domains": ["creative"] },
        { "id": "content", "name": "Content Agent", "name_zh": "内容生产",
          "role": "文案、图片、视频、脚本等内容生成",
          "steps": ["create"], "domains": ["content"] },
        { "id": "critic_compliance", "name": "Critic & Compliance Agent", "name_zh": "审核与合规",
          "role": "内容质量、事实核查、原创性、品牌一致性、合规审查",
          "steps": ["review", "execute"], "domains": ["quality_compliance", "publishing"] },
        { "id": "analytics_learning", "name": "Analytics & Learning Agent", "name_zh": "数据分析与学习",
          "role": "分析发布表现、提取经验、实验、优化策略",
          "steps": ["measure", "learn"], "domains": ["analytics", "learning", "memory"] },
    ])
}

/// 运营周期 10 步的稳定顺序 (动画按此顺序高亮)。
fn cycle_steps_model() -> Value {
    json!([
        "observe",
        "understand",
        "decide",
        "plan",
        "create",
        "review",
        "execute",
        "measure",
        "learn",
        "adapt"
    ])
}

async fn architecture(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
) -> Json<Value> {
    let layers = json!([
        {
            "id": "client",
            "title": "客户端 Client",
            "nodes": [
                node("console", "Web Console", "Next.js 15 · localhost:3000",
                     "登录/总览/任务/设备/AI 生成/急停/指标/数字员工/世界情报(信源健康审计+趋势情报)/架构图; JWT 存 localStorage"),
                node("agent", "Android Agent (手机 APK)", "com.sincy.agent · Kotlin + Rust core(JNI) · WSS client",
                     "配对后连 /agent/connect, 拉取签名任务简报, 无障碍服务感知节点树, dispatchGesture 多段连续 stroke 注入真实触摸执行 adapter, 挑战屏停 WAITING_USER, 上报结果与证据"),
                node("adbio", "PC 端 adb 模拟触屏", "sincy-agent-sim --io adb [--adb-device X] [--seed N] · AdbIo",
                     "电脑驱动 USB 真机/模拟器: uiautomator dump 感知 + `adb shell input tap/swipe/text` 注入模拟触屏; 与 APK 走同一套 humanize 手势计划"),
                node("sim", "Sim Agent (脚本化验证)", "sincy-agent-sim --io scripted · ScriptedIo",
                     "脚本化屏幕快照驱动同一套 adapter 解释器, 用于本地/CI 验证设备路由"),
            ]
        },
        {
            "id": "gateway",
            "title": "接入层 cloud-api (Axum)",
            "nodes": [
                node("api", "REST API /api/v1", "apps/cloud-api",
                     "auth · agencies · clients/brands(语言设置) · packages · tasks/publish · devices 配对 · kill-switches · analytics/metrics · ai/generate · employees(周期/调度) · skills · intel(sources/providers/route/ingest/items/opportunities/jobs/collect/stats/health/trends) · architecture"),
                node("wss", "Agent WSS Gateway", "/api/v1/agent/connect",
                     "设备 token 鉴权; 每 2s 派发最多 5 个 QUEUED 任务(最老优先); 接收执行结果"),
                node("mw", "Middleware", "auth_middleware · CORS · request_id",
                     "JWT(60min) 校验注入 AuthCtx; 统一错误体 {error{code,kind,message,request_id}}; 公开路径白名单"),
            ]
        },
        {
            "id": "domain",
            "title": "业务域 crates",
            "nodes": [
                node("de", "数字员工 sincy-employee", "lifecycle · cycle · agents(6) · 9 技能域/77 skills · memory",
                     "状态机 CREATED→READY→RUNNING→PAUSED→STOPPED; 10 步运营周期(可恢复, 步级幂等, 每步记录调用的 agent+skills); 后台调度器按 strategy.cadence_per_week 自动触发 scheduled 周期; 6 核心 agent 按域动态调用 skill; 版本化身份/使命/策略/skill; 按品牌 language 设置生成内容; 结构化记忆+洞察"),
                node("wi", "世界情报 sincy-intel", "registry · router · opportunity · adapter",
                     "信源/Provider 注册表; 路由打分 健康.45+可靠.30+限流.15+成本.10 + 降级链; ingest 按 dedup_key 去重; 机会引擎; DE 适配器(只给建议不改策略)"),
                node("pub", "发布管线 routing/execution/platform/content", "publish_core",
                     "急停 → 审批门(pending 包)→ 路由选择(device@10/api@50/assisted@90)→ 配额 → 幂等键 → 审计; 任务租约/到期回收; side_effect 后禁止自动重试"),
                node("adapter", "Adapter 引擎 sincy-adapter", "YAML 定义 · 解释器 · 漂移检测 · HMAC 签名",
                     "x.publish_post v1: home→composer→set_text→Post(side effect)→posted_confirm 验证; 锚点匹配率判漂移 UiDrift"),
                node("device", "设备管理 sincy-device", "配对 · 会话",
                     "challenge-response: proof=sha256(sha256(secret)+nonce), secret 只存哈希; 长期 device token"),
                node("collector", "情报采集器 intel_collector", "tokio 定时任务(默认 30min) · WI §9",
                     "32 条默认 feed: arXiv×5 / HN Algolia×3 / dev.to×4 / Wikipedia RC / GitHub 趋势×3 / YouTube×3 / IT之家/少数派/掘金/V2EX RSS / Google News×5(en+zh) / Product Hunt×2 / B站排行榜×2; 可选: GDELT(SINCY_INTEL_GDELT=1)、自托管 RSSHub 扩展×8(Reddit/知乎/微博/抖音/百度热榜/36氪)、SearXNG 自托管×2; 归一化 WI §11 入 intel_items(dedup_key 去重), jobs 记 last_fetched/last_new; 失败记 jobs 不强攻; 红线: 只走公开只读 API, 礼貌间隔, 不绕认证/验证码/限流"),
                node("wihealth", "Source Health Audit", "GET /intel/health",
                     "按 source 聚合 job 运行记录: healthy(有新入库)/ degraded(在跑但 0 新增或部分失败)/ failed(全部失败)/ not_configured(已注册但 job 暂停、待自托管实例/密钥/出口 IP)/ disabled; 注册 ≠ 可用, 审计才是真相"),
                node("witrend", "Trend Intelligence", "GET /intel/trends",
                     "intel_items 最近 24h 按 topic 聚合: 近 6h 速度 velocity/h, 相对前 18h 的 growth%, 跨平台数(platforms/sources), 样本标题; 输出 Trend(topic,growth,velocity,跨平台传播) 供机会引擎与数字员工"),
                node("humanize", "人工化手势 humanize", "sincy-adapter::humanize · 确定性 SplitMix64 RNG",
                     "共享手势计划器: tap 偏心高斯抖动(不出 10% 内缩框)+ 随机 hold/pre/post 延迟; swipe 二次贝塞尔弓形 + 逐点抖动 + ease-in-out 分段时序(中段快两端慢)+ 噪声; 每段带 pressure 模型(后端能写入才生效); APK 与 adb 两端执行同一份计划, --seed 可复现"),
                node("ai", "AI sincy-ai", "LLM gateway · budget · analytics",
                     "SINCY_LLM_PROVIDER: mock(仅本地) / openai-compatible(任意兼容端点, 含 Mistral api.mistral.ai/v1, key 走 SINCY_LLM_API_KEY 环境变量不入库) / manual-file(本地把 Vibe 会话接入为 LLM: prompt 写文件等回写); 月度预算检查+熔断(FR-026); 成本落账; 指标回流内容版本(FR-016)"),
            ]
        },
        {
            "id": "data",
            "title": "数据层",
            "nodes": [
                node("db", "PostgreSQL", "migrations 0001–0021 · sqlx",
                     "agencies/members/clients/brands · publishing_packages · device_tasks/task_events/execution_routes · devices/sessions · kill_switches · quota · channel_capabilities/app_adapter_versions · digital_employees/cycles/tasks/memory/insights · skills/skill_versions/employee_skill_bindings · intelligence_sources/providers/items/opportunities; RLS 租户隔离"),
                node("vault", "Vault (密钥)", "vault:// 引用",
                     "平台/情报源的 API key 只存引用, 不落库明文"),
            ]
        },
        {
            "id": "external",
            "title": "外部系统",
            "nodes": [
                node("x", "X (Twitter) 平台", "device 自动化(免费, ADR-016)",
                     "手机 APK (dispatchGesture 真实触摸) 或 PC adb input 驱动真实 app 完成发布; 付费 API 路由已保留默认不启用; 不绕认证/验证码/限流, 挑战屏停人工"),
                node("llm", "LLM Provider", "OpenAI-compatible API (Mistral 等)",
                     "SINCY_LLM_PROVIDER=openai-compatible + SINCY_LLM_API_KEY 环境变量; key 不入库不进 git; 本地可 mock / manual-file"),
                node("wisrc", "情报信源(只读公共, 26 个注册源)", "arXiv · HN · dev.to · Wikipedia · Google News · Product Hunt · B站 · GitHub · YouTube · V2EX · 掘金 · 少数派 · IT之家 · RSSHub(self-host) · SearXNG(self-host)",
                     "采集走各自公开只读接口; 中文热榜/Reddit 走自托管 RSSHub; GDELT/Google Trends 待专用出口; 付费 provider(TikHub/SocialDataX/Firecrawl/reach-mcp) key 只放 vault, 未配置即 not_configured"),
            ]
        },
        {
            "id": "prod",
            "title": "生产环境 (SINCY_ENV=production · 本地 sincy_prod 已可验证, 云端 Vercel+Supabase 见 docs/deploy.md)",
            "planned": true,
            "nodes": [
                node("proddb", "生产库 sincy_prod / Supabase", "同一套 migrations + RLS; sincy_app 非超级用户",
                     "本地生产库已启用(SINCY_ENV=production, 独立 DB, 强制禁 mock); 云端用 Supabase pooler; 首管理员由 SINCY_BOOTSTRAP_ADMIN_* 幂等创建, 无内置 demo 账号"),
                node("vercel", "Vercel 托管前端", "frontend/vercel.json · Next.js 生产构建",
                     "SINCY_API_INTERNAL 指向 cloud-api; rewrites 同域代理 /api/v1, 零 CORS; 域名+HTTPS 由 Vercel 接管"),
                node("worker", "云端 cloud-api", "容器 · Fly/Render/Railway (WSS 长连接, 勿用 serverless)",
                     "SINCY_ENV=production 启动即强制: 真 JWT secret / 禁 mock adapter / LLM mock+manual-file 拒绝 / healthz 暴露 env; 跑 REST/WSS/采集器/员工调度器"),
                node("devfarm", "云端真机农场", "adb over TCP · 手机 APK agent",
                     "生产环境设备不离云: 手机装 APK 后 adb forward 到 worker, AdbIo --io adb 驱动, humanize 手势, 挑战屏通知人工远程处理"),
            ]
        },
    ]);

    let edges = json!([
        edge("console", "api", "REST /api/v1 + Bearer JWT", "http"),
        edge("api", "mw", "每请求 JWT 校验 → AuthCtx", "internal"),
        edge("agent", "wss", "WSS connect (device token)", "ws"),
        edge("wss", "agent", "device.task.assign (≤5/2s)", "ws"),
        edge("agent", "wss", "执行结果/证据上报", "ws"),
        edge(
            "agent",
            "api",
            "GET /agent/tasks/{id} (device token)",
            "http"
        ),
        edge(
            "console",
            "api",
            "POST /devices/register/start (JWT)",
            "http"
        ),
        edge(
            "agent",
            "api",
            "POST /devices/register/complete (公开, proof)",
            "http"
        ),
        edge(
            "api",
            "de",
            "POST /employees/{id}/cycles 触发周期",
            "internal"
        ),
        edge(
            "de",
            "wi",
            "opportunities 经 DE 适配器 (WI §20)",
            "internal"
        ),
        edge("de", "ai", "Content Factory port: LLM 起草", "internal"),
        edge(
            "de",
            "pub",
            "save_package/approve/publish —— 与人工同一 publish_core",
            "internal"
        ),
        edge("console", "api", "packages 审批 approve (FR-024)", "http"),
        edge(
            "api",
            "pub",
            "POST /tasks/publish → publish_core",
            "internal"
        ),
        edge("pub", "wss", "路由=device → 任务入队", "internal"),
        edge(
            "agent",
            "adapter",
            "执行签名 adapter (x.publish_post)",
            "internal"
        ),
        edge("adapter", "x", "驱动手机上的 X app 发布", "external"),
        edge("pub", "db", "device_tasks / routes / events", "data"),
        edge("de", "db", "cycles/tasks/memory/insights", "data"),
        edge("wi", "db", "intel_items / opportunities", "data"),
        edge("api", "db", "全部业务读写 (RLS 按 agency 隔离)", "data"),
        edge("wi", "wisrc", "只读采集/搜索/趋势", "external"),
        edge(
            "collector",
            "wisrc",
            "定时拉取注册的只读公开源 (32 条默认 feed + 自托管扩展)",
            "external"
        ),
        edge(
            "collector",
            "db",
            "intel_items 归一化入库 (dedup_key 去重) + jobs last_fetched/last_new",
            "data"
        ),
        edge("collector", "wi", "新信号触发机会引擎重算", "internal"),
        edge(
            "api",
            "collector",
            "POST /intel/collect 手动触发一轮",
            "internal"
        ),
        edge(
            "collector",
            "wihealth",
            "jobs 运行记录 → 健康审计",
            "internal"
        ),
        edge(
            "wi",
            "witrend",
            "intel_items 24h 聚合 → 趋势(velocity/growth/跨平台)",
            "internal"
        ),
        edge("witrend", "wi", "趋势信号喂机会引擎", "internal"),
        edge(
            "adapter",
            "humanize",
            "tap/scroll → 人工化手势计划(坐标+时序)",
            "internal"
        ),
        edge(
            "humanize",
            "agent",
            "计划 JSON 下发 → dispatchGesture 连续多段 stroke",
            "internal"
        ),
        edge(
            "humanize",
            "adbio",
            "同一份计划 → adb input 分段执行",
            "internal"
        ),
        edge(
            "adbio",
            "x",
            "adb shell input tap/swipe 注入模拟触屏",
            "external"
        ),
        edge("ai", "llm", "generate (key 走环境变量)", "external"),
        edge("ai", "db", "成本落账/预算熔断", "data"),
        edge("device", "vault", "secret 哈希与凭证引用", "external"),
        edge(
            "api",
            "vault",
            "adapter 签名密钥 / 情报源 key 引用",
            "external"
        ),
        edge("console", "vercel", "生产前端入口", "http"),
        edge("vercel", "worker", "REST/WSS → cloud-api", "http"),
        edge("worker", "proddb", "生产库 (同 migrations + RLS)", "data"),
        edge("worker", "devfarm", "adb-over-TCP 派发 device 任务", "ws"),
        edge("devfarm", "x", "云端真机执行发布", "external"),
    ]);

    let flows = json!([
        {
            "id": "F1", "title": "登录鉴权",
            "steps": "Console → POST /auth/login(email+password) → JwtIssuer 签发 60min JWT → 每个请求 Bearer → auth_middleware 校验并注入 AuthCtx(agency_id/roles) → 按 agency 作租户边界"
        },
        {
            "id": "F2", "title": "手机设备配对",
            "steps": "Console POST /devices/register/start(JWT) → 拿到一次性 device_secret → 手机端 proof=sha256(sha256(secret)+nonce) → POST /devices/register/complete(公开) → 激活设备并换取长期 device token → WSS /agent/connect 建立长连接"
        },
        {
            "id": "F3", "title": "内容生产 → 审批 → 发布 → 设备执行 → 指标回流",
            "steps": "AI 生成(/ai/generate 存包) 或数字员工产出 → publishing_packages(pending) → 人工 approve → POST /tasks/publish → publish_core: 急停检查 → 审批门 → 路由(device@10 优先已配对设备) → 配额检查 → 幂等键去重 → 写 device_tasks QUEUED → WSS 派发 → agent 执行 x.publish_post adapter(含 side effect 与 posted_confirm 验证) → COMPLETED → /analytics/metrics 回流指标"
        },
        {
            "id": "F4", "title": "数字员工运营周期 (10 步, 步级幂等可恢复)",
            "steps": "POST /employees/{id}/cycles → observe → understand(WI 机会) → decide(score≥0.30 选最高) → plan → create(LLM 起草) → review(critic: 280 字/查重/边界) → execute(存包; auto_publish=false 则停 pending 待人审) → measure(查 task state, 不轻信点击) → learn(写 memory+insight) → adapt(策略调整建议, 需审批)"
        },
        {
            "id": "F5", "title": "世界情报采集 → 路由 → 机会",
            "steps": "信源注册表(intelligence_sources, 26 个注册源, 迁移 0017/0019/0022) → collector 定时拉取(默认 30min 一轮, 32 条默认 feed) → 归一化入 intel_items(dedup_key 去重) → POST /intel/route 按 健康/可靠/限流/成本 打分出 provider 降级链 → 机会引擎打分 → intel_opportunities → 数字员工经 DE 适配器消费"
        },
        {
            "id": "F6", "title": "信源健康审计 (Source Health Audit)",
            "steps": "GET /intel/health 聚合 intelligence_sources × intel_collection_jobs × intel_items → 每个 source 出 healthy / degraded(在跑但 0 新增)/ failed / not_configured(注册但 job 暂停, 待自托管 RSSHub、vault key 或专用出口 IP)/ disabled → 控制台情报页红黄绿展示, 注册不等于可用"
        },
        {
            "id": "F7", "title": "趋势情报 (Trend Intelligence)",
            "steps": "GET /intel/trends 把 intel_items 最近 24h 按 topic 聚合 → 近 6h 速度 velocity/h + 相对前 18h 增长率 growth% + 跨平台数/来源列表 + 样本标题 → 喂机会引擎(WI §19)和数字员工选题, 例如 “AI Agent 在 GitHub/HN 先爆, 2h 后跟进 X”"
        },
        {
            "id": "F8", "title": "设备端人工化触摸执行 (APK / adb 两路)",
            "steps": "解释器产出 Action → sincy-adapter::humanize 生成手势计划(偏心抖动 tap / 贝塞尔弓形+逐点抖动+ease-in-out 分段 swipe, 随机延迟, pressure 模型) → 手机 APK: JNI 下发计划, 无障碍 dispatchGesture 多段 willContinue 连续 stroke 注入真实触摸; 电脑: AdbIo 执行 `adb shell input tap/swipe` 分段 → 挑战屏/漂移/版本不符即停 (WAITING_USER / UiDrift), 不盲点"
        },
        {
            "id": "F9", "title": "生产化路径 (SINCY_ENV=production)",
            "steps": "本地先起 sincy_prod 生产库验证(bootstrap 首 admin, 强制禁 mock) → 前端部署 Vercel(vercel.json, SINCY_API_INTERNAL 代理 /api/v1) → cloud-api 容器部署到支持 WSS 的平台 → Supabase 托管 Postgres(同 migrations + RLS, sincy_app 角色) → 真机 APK 经 adb-over-TCP 接入云端设备农场 → healthz 暴露 env/mock 状态, key 全走 vault/环境变量, 不入库不进 git"
        },
    ]);

    // skill 域统计 (9 域 + 数量 + 名称样例) — 实时查库
    let skill_domains = match state.pool() {
        Ok(pool) => {
            let rows = sqlx::query(
                "SELECT domain, count(*)::int8 AS count, array_agg(name ORDER BY name) AS names \
                 FROM skills WHERE status = 'active' GROUP BY domain ORDER BY domain",
            )
            .fetch_all(pool)
            .await
            .unwrap_or_default();
            let mut items: Vec<Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "id": r.get::<String, _>("domain"),
                        "count": r.get::<i64, _>("count"),
                        "skills": r.get::<Vec<String>, _>("names"),
                    })
                })
                .collect();
            let total: i64 = items.iter().map(|d| d["count"].as_i64().unwrap_or(0)).sum();
            // 保证 9 个域都出现 (即使还没有 seed)
            for d in sincy_employee::skills::SkillDomain::ALL {
                if !items.iter().any(|i| i["id"] == d.as_str()) {
                    items.push(json!({ "id": d.as_str(), "title_zh": d.title_zh(), "count": 0, "skills": [] }));
                } else if let Some(i) = items.iter_mut().find(|i| i["id"] == d.as_str()) {
                    i["title_zh"] = json!(d.title_zh());
                }
            }
            json!({ "domains": items, "total_skills": total })
        }
        Err(_) => json!({ "domains": [], "total_skills": 0 }),
    };

    // runtime: 本租户最近一个运营周期及其 step 落痕 (架构图动画高亮用)
    let runtime = match state.pool() {
        Ok(pool) => {
            let cyc = sqlx::query(
                "SELECT c.id, c.employee_id, e.name AS employee_name, c.state, c.trigger, c.created_at \
                 FROM operating_cycles c JOIN digital_employees e ON e.id = c.employee_id \
                 WHERE e.agency_id = $1 ORDER BY c.created_at DESC LIMIT 1",
            )
            .bind(auth.agency_id.as_uuid())
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
            match cyc {
                Some(r) => {
                    let cycle_id: uuid::Uuid = r.get("id");
                    let steps = sqlx::query(
                        "SELECT agent, step, state, output, created_at FROM cycle_tasks \
                         WHERE cycle_id = $1 ORDER BY created_at",
                    )
                    .bind(cycle_id)
                    .fetch_all(pool)
                    .await
                    .unwrap_or_default();
                    json!({
                        "cycle_id": cycle_id,
                        "employee_id": r.get::<uuid::Uuid, _>("employee_id"),
                        "employee_name": r.get::<String, _>("employee_name"),
                        "state": r.get::<String, _>("state"),
                        "trigger": r.get::<String, _>("trigger"),
                        "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
                        "steps": steps.iter().map(|s| json!({
                            "agent": s.get::<String, _>("agent"),
                            "step": s.get::<String, _>("step"),
                            "state": s.get::<String, _>("state"),
                            "skills": s.get::<Option<Value>, _>("output").and_then(|o| o.get("skills").cloned()),
                            "skill": s.get::<Option<Value>, _>("output").and_then(|o| o.get("skill").cloned()),
                            "created_at": s.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
                        })).collect::<Vec<_>>(),
                    })
                }
                None => json!(null),
            }
        }
        Err(_) => json!(null),
    };

    Json(json!({
        "title": "Sincy 系统架构 (as-built)",
        "service": "cloud-api",
        "version": env!("CARGO_PKG_VERSION"),
        "generated_at": chrono::Utc::now(),
        "layers": layers,
        "edges": edges,
        "flows": flows,
        "agents": agents_model(),
        "cycle_steps": cycle_steps_model(),
        "skill_domains": skill_domains,
        "runtime": runtime,
        "edge_kinds": {
            "http": "HTTP REST (JWT)",
            "ws": "WebSocket",
            "internal": "内部调用",
            "data": "数据读写",
            "external": "外部系统",
        }
    }))
}
