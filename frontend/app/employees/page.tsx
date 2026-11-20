"use client";

import { useCallback, useEffect, useState } from "react";
import {
  LANGUAGES,
  api,
  getToken,
  type BrandItem,
  type CycleDetail,
  type CycleItem,
  type CycleOutcome,
  type EmployeeItem,
  type InsightItem,
  type MemoryItem,
  type SkillItem,
} from "@/lib/api";

// 数字员工页 (DE guide): 创建员工 -> 启动 -> 手动触发运营周期 ->
// 查看周期步骤 / 记忆 / 洞察。auto_publish=false 时生成的内容包停在
// 待审批 (FR-024), 由任务页人工批准后发布。
export default function EmployeesPage() {
  const [employees, setEmployees] = useState<EmployeeItem[]>([]);
  const [brands, setBrands] = useState<BrandItem[]>([]);
  const [selected, setSelected] = useState("");
  const [cycles, setCycles] = useState<CycleItem[]>([]);
  const [cycleDetail, setCycleDetail] = useState<CycleDetail | null>(null);
  const [memory, setMemory] = useState<MemoryItem[]>([]);
  const [insights, setInsights] = useState<InsightItem[]>([]);
  const [skills, setSkills] = useState<SkillItem[]>([]);
  const [name, setName] = useState("");
  const [brandId, setBrandId] = useState("");
  const [language, setLanguage] = useState("zh-CN");
  const [autoPublish, setAutoPublish] = useState(false);
  const [lastCycle, setLastCycle] = useState<CycleOutcome | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const loadEmployees = useCallback(async () => {
    if (!getToken()) return;
    try {
      const [e, b] = await Promise.all([api.listEmployees(), api.listBrands()]);
      setEmployees(e.items);
      setBrands(b.items);
      setSelected((cur) => (e.items.some((x) => x.id === cur) ? cur : (e.items[0]?.id ?? "")));
      if (!brandId && b.items.length > 0) setBrandId(b.items[0].id);
    } catch (e) {
      setError(String(e));
    }
  }, [brandId]);

  const loadDetail = useCallback(async (id: string) => {
    if (!id || !getToken()) return;
    try {
      const [c, m, i, sk] = await Promise.all([
        api.listCycles(id),
        api.listMemory(id),
        api.listInsights(id),
        api.listEmployeeSkills(id),
      ]);
      setCycles(c.items);
      setMemory(m.items);
      setInsights(i.items);
      setSkills(sk.items);
      setCycleDetail(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    loadEmployees();
  }, [loadEmployees]);

  useEffect(() => {
    loadDetail(selected);
  }, [selected, loadDetail]);

  async function act(fn: () => Promise<unknown>, refresh = true) {
    setBusy(true);
    setError(null);
    try {
      await fn();
      if (refresh) {
        await loadEmployees();
        await loadDetail(selected);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <main>
      <h1>数字员工</h1>
      <p className="muted">
        数字员工按"感知→理解→决策→计划→创作→审核→执行→衡量→学习→适应"的运营周期工作;
        内容包默认需人工批准后才发布 (auto_publish 关闭时)。
      </p>
      {error && <p style={{ color: "var(--danger)" }}>{error}</p>}

      <section className="panel">
        <h2>创建员工</h2>
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          <input placeholder="姓名 (如 Aria)" value={name} onChange={(e) => setName(e.target.value)} />
          <select
            value={brandId}
            onChange={(e) => {
              setBrandId(e.target.value);
              const b = brands.find((x) => x.id === e.target.value);
              if (b?.language) setLanguage(b.language); // 品牌语言作为员工默认输出语言
            }}
          >
            {brands.length === 0 && <option value="">先创建品牌</option>}
            {brands.map((b) => (
              <option key={b.id} value={b.id}>{b.name} · {b.language}</option>
            ))}
          </select>
          <select value={language} onChange={(e) => setLanguage(e.target.value)} title="内容输出语言">
            {LANGUAGES.map((l) => (
              <option key={l.code} value={l.code}>{l.label}</option>
            ))}
          </select>
          <label>
            <input type="checkbox" checked={autoPublish} onChange={(e) => setAutoPublish(e.target.checked)} />
            自动发布
          </label>
          <button
            disabled={busy || !name || !brandId}
            onClick={() =>
              act(async () => {
                const r = await api.createEmployee({
                  name,
                  role: "content_operator",
                  brand_id: brandId,
                  channel: "x",
                  auto_publish: autoPublish,
                  identity: { name, role: "content_operator", voice: "neutral", language, audience: ["developers"], expertise: ["ai agents"], values: [], boundaries: [] },
                  mission: { statement: "在 X 上建立品牌在 AI agents 领域的权威", domain: "content", audience: ["developers"], value_proposition: "实用、不灌水", constraints: ["280 字以内"], success_definition: "互动增长" },
                  strategy: { pillars: ["ai agents", "llm ops", "multi-agent"], cadence_per_week: 7, formats: ["text"], risk_tolerance: 0.3, exploration_budget: 0.2 },
                });
                setSelected(r.id);
                setName("");
              })
            }
          >
            创建
          </button>
        </div>
      </section>

      <section className="panel">
        <h2>员工列表</h2>
        {employees.length === 0 && <p className="muted">还没有数字员工。</p>}
        {employees.map((e) => (
          <div key={e.id} style={{ display: "flex", gap: 8, alignItems: "center", padding: "6px 0", flexWrap: "wrap" }}>
            <button
              style={{ fontWeight: selected === e.id ? "bold" : "normal", border: selected === e.id ? "1px solid var(--accent)" : "1px solid var(--border)" }}
              onClick={() => setSelected(e.id)}
            >
              {e.name}
            </button>
            <span className="muted">{e.status} · {e.channel} · {e.language ?? "zh-CN"} · auto_publish={String(e.auto_publish)}</span>
            <select
              value={e.language ?? "zh-CN"}
              disabled={busy}
              title="内容输出语言"
              onChange={(ev) => act(() => api.patchEmployee(e.id, { language: ev.target.value }))}
            >
              {LANGUAGES.map((l) => (
                <option key={l.code} value={l.code}>{l.code}</option>
              ))}
            </select>
            {e.status === "READY" && <button disabled={busy} onClick={() => act(() => api.employeeAction(e.id, "start"))}>启动</button>}
            {e.status === "RUNNING" && <button disabled={busy} onClick={() => act(() => api.employeeAction(e.id, "pause"))}>暂停</button>}
            {e.status === "PAUSED" && <button disabled={busy} onClick={() => act(() => api.employeeAction(e.id, "resume"))}>恢复</button>}
            {(e.status === "RUNNING" || e.status === "PAUSED") && <button disabled={busy} onClick={() => act(() => api.employeeAction(e.id, "stop"))}>停止</button>}
            <button
              disabled={busy}
              onClick={() => act(async () => api.patchEmployee(e.id, { auto_publish: !e.auto_publish }))}
            >
              {e.auto_publish ? "关闭自动发布" : "开启自动发布"}
            </button>
            {e.status === "RUNNING" && (
              <button
                disabled={busy}
                onClick={() =>
                  act(async () => {
                    const r = await api.runCycle(e.id, "manual");
                    setLastCycle(r);
                  })
                }
              >
                运行一个周期
              </button>
            )}
          </div>
        ))}
        {lastCycle && (
          <p className="muted">
            最新周期 {lastCycle.cycle_id.slice(0, 8)}: {lastCycle.state} · 机会 {lastCycle.selected_opportunity ?? "无"} ·
            内容包 {lastCycle.package_id ? lastCycle.package_id.slice(0, 8) : "无"} ·
            任务 {lastCycle.task_id ? `${lastCycle.task_id.slice(0, 8)}(${lastCycle.task_state})` : "待审批"}
          </p>
        )}
      </section>

      {selected && (
        <>
          <section className="panel">
            <h2>运营周期</h2>
            {cycles.length === 0 && <p className="muted">还没有周期。</p>}
            {cycles.map((c) => (
              <div key={c.id} style={{ padding: "4px 0" }}>
                <button
                  style={{ border: "1px solid var(--border)" }}
                  onClick={() => act(async () => setCycleDetail(await api.getCycle(c.id)), false)}
                >
                  {c.id.slice(0, 8)}
                </button>{" "}
                <span className="muted">
                  {c.state} · {c.trigger} · {c.created_at.slice(0, 19).replace("T", " ")}
                  {c.summary && typeof c.summary === "object" && "topic" in c.summary ? ` · topic=${String((c.summary as Record<string, unknown>).topic)}` : ""}
                </span>
              </div>
            ))}
            {cycleDetail && (
              <table style={{ marginTop: 8 }}>
                <thead><tr><th>步骤</th><th>agent</th><th>状态</th></tr></thead>
                <tbody>
                  {cycleDetail.tasks.map((t) => (
                    <tr key={t.id}><td>{t.step}</td><td>{t.agent}</td><td>{t.state}</td></tr>
                  ))}
                </tbody>
              </table>
            )}
          </section>

          <section className="panel">
            <h2>记忆</h2>
            {memory.length === 0 && <p className="muted">暂无记忆。</p>}
            {memory.slice(0, 10).map((m) => (
              <p key={m.id} className="muted">{m.type}: {m.content} (conf={m.confidence})</p>
            ))}
          </section>

          <section className="panel">
            <h2>洞察</h2>
            {insights.length === 0 && <p className="muted">暂无洞察。</p>}
            {insights.slice(0, 10).map((i) => (
              <p key={i.id} className="muted">{i.content}</p>
            ))}
          </section>

          <section className="panel">
            <h2>绑定的 Skills ({skills.filter((s) => s.enabled !== false).length}/{skills.length})</h2>
            <p className="muted" style={{ marginTop: 0 }}>
              创建员工时自动绑定全部 active skills; 每个周期由各 agent 按域动态调用, 停用后该 skill 不再被调用。
            </p>
            {Object.entries(
              skills.reduce<Record<string, SkillItem[]>>((acc, s) => {
                (acc[s.domain] ??= []).push(s);
                return acc;
              }, {}),
            ).map(([domain, list]) => (
              <div key={domain} style={{ marginBottom: 8 }}>
                <strong style={{ fontSize: 13 }}>{domain}</strong>
                <div style={{ display: "flex", flexWrap: "wrap", gap: 6, marginTop: 4 }}>
                  {list.map((s) => (
                    <button
                      key={s.id}
                      data-skill={s.name}
                      disabled={busy}
                      title={`${s.title} v${s.version} — ${s.description}${s.tools?.length ? `\ntools: ${s.tools.join(", ")}` : ""}`}
                      onClick={() => act(() => api.bindSkill(selected, { skill_id: s.id, enabled: s.enabled === false }))}
                      style={{
                        fontSize: 11.5,
                        padding: "3px 10px",
                        borderRadius: 999,
                        border: "1px solid var(--border)",
                        background: s.enabled === false ? "transparent" : "rgba(76,141,255,0.12)",
                        opacity: s.enabled === false ? 0.45 : 1,
                        cursor: "pointer",
                      }}
                    >
                      {s.name} v{s.version}{s.enabled === false ? " (已停用)" : ""}
                    </button>
                  ))}
                </div>
              </div>
            ))}
          </section>
        </>
      )}
    </main>
  );
}
