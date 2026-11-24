"use client";

// 🧑‍🚀 数字员工: 员工列表 + 本周期 10 步 + 策略配置(可改,落库新版本) + 粉丝轨迹 + 学习实验 + 技能

import { useEffect, useState } from "react";

import {
  api,
  type BrandItem,
  type IndustryDef,
  type AccountMetricAccount,
  type CycleDetail,
  type CycleItem,
  type EmployeeItem,
  type IntelBoard,
  type InsightItem,
  type StrategyVersion,
} from "@/lib/api";
import { Badge, Err, Loading, Spark, fmtDelta } from "@/components/ui";

const STEPS = ["observe", "understand", "decide", "plan", "create", "review", "execute", "measure", "learn", "adapt"];
const STEP_ZH: Record<string, string> = {
  observe: "观察", understand: "理解", decide: "决策", plan: "计划", create: "创作",
  review: "审核", execute: "执行", measure: "衡量", learn: "学习", adapt: "适应",
};

export function EmployeesView() {
  const [emps, setEmps] = useState<EmployeeItem[] | null>(null);
  const [sel, setSel] = useState<string | null>(null);
  const [cycles, setCycles] = useState<CycleItem[] | null>(null);
  const [detail, setDetail] = useState<CycleDetail | null>(null);
  const [strat, setStrat] = useState<StrategyVersion | null>(null);
  const [insights, setInsights] = useState<InsightItem[] | null>(null);
  const [metrics, setMetrics] = useState<AccountMetricAccount[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  // 策略编辑表单
  const [fTarget, setFTarget] = useState<string>("");
  const [fCadence, setFCadence] = useState<string>("7");
  const [fExplore, setFExplore] = useState<string>("0.2");
  const [fAuto, setFAuto] = useState(false);
  const [fPillars, setFPillars] = useState<string>("");
  // 编辑/删除员工
  const [editing, setEditing] = useState(false);
  const [eName, setEName] = useState("");
  const [eRole, setERole] = useState("");
  const [eChannel, setEChannel] = useState("");
  const [eVoice, setEVoice] = useState("");
  const [eFocus, setEFocus] = useState("");
  const [eMission, setEMission] = useState("");
  // 新建员工表单
  const [creating, setCreating] = useState(false);
  const [brands, setBrands] = useState<BrandItem[]>([]);
  const [industries, setIndustries] = useState<IndustryDef[]>([]);
  const [cName, setCName] = useState("");
  const [cBrand, setCBrand] = useState("");
  const [cChannel, setCChannel] = useState("x");
  const [cScope, setCScope] = useState("niche");
  const [cIndustry, setCIndustry] = useState("");
  const [cVerticals, setCVerticals] = useState("");
  const [cTopics, setCTopics] = useState("");
  const [cExcluded, setCExcluded] = useState("");

  useEffect(() => {
    api.listEmployees().then((d) => {
      setEmps(d.items);
      if (d.items.length > 0) setSel((s) => s ?? d.items[0].id);
    }).catch((e) => setErr(String(e)));
    api.listAccountMetricsRecent().then((d) => setMetrics(d.accounts)).catch(() => {});
    api.listBrands().then((d) => { setBrands(d.items); if (d.items[0]) setCBrand((b) => b || d.items[0].id); }).catch(() => {});
    api.listIndustries().then((d) => setIndustries(d.items)).catch(() => {});
  }, []);

  useEffect(() => {
    if (!sel) return;
    setCycles(null); setDetail(null); setStrat(null); setInsights(null); setSaved(null);
    api.listCycles(sel, 10).then((d) => {
      setCycles(d.items);
      if (d.items[0]) api.getCycle(d.items[0].id).then(setDetail).catch(() => {});
    }).catch((e) => setErr(String(e)));
    api.listStrategyVersions(sel).then((d) => {
      const act = d.items.find((x) => x.status === "ACTIVE") ?? d.items[0] ?? null;
      setStrat(act);
      if (act) {
        setFTarget(act.strategy.target_followers != null ? String(act.strategy.target_followers) : "");
        setFCadence(String(act.strategy.cadence_per_week ?? 7));
        setFExplore(String(act.strategy.exploration_budget ?? 0.2));
        setFPillars((act.strategy.pillars ?? []).join(", "));
      }
    }).catch(() => {});
    api.listInsights(sel).then((d) => setInsights(d.items.slice(0, 5))).catch(() => {});
    api.getEmployee(sel).then((d) => { const idn = (d as Record<string, unknown>).identity as Record<string, unknown> | undefined; if (idn) setEVoice(String(idn.voice ?? "")); }).catch(() => {});
    const emp = emps?.find((e) => e.id === sel);
    if (emp) { setFAuto(emp.auto_publish); setEName(emp.name); setERole(emp.role); setEChannel(emp.channel); setEFocus(JSON.stringify(emp.focus ?? { scope: "general" }, null, 2)); }
  }, [sel]); // eslint-disable-line react-hooks/exhaustive-deps

  if (err) return <Err e={err} />;
  if (!emps) return <Loading />;
  const emp = emps.find((e) => e.id === sel) ?? null;
  const myMetric = metrics.find((m) => m.employee_id === sel);
  const target = strat?.strategy.target_followers ?? null;
  const now = myMetric?.followers ?? null;
  const progress = target && now ? Math.min(100, Math.round((now / target) * 100)) : null;

  const saveStrategy = async () => {
    if (!sel || !strat) return;
    setSaved(null);
    try {
      const next = {
        ...strat.strategy,
        cadence_per_week: Number(fCadence) || 7,
        exploration_budget: Number(fExplore) || 0.2,
        target_followers: fTarget.trim() === "" ? null : Number(fTarget),
        pillars: fPillars.split(",").map((s) => s.trim()).filter(Boolean),
      };
      await api.createStrategyVersion(sel, next);
      if (emp) await api.patchEmployee(sel, { auto_publish: fAuto });
      setSaved("已保存为新策略版本 ✅");
    } catch (e) { setSaved(`保存失败: ${e}`); }
  };

  const createEmp = async () => {
    setSaved(null);
    try {
      const r = await api.createEmployee({
        name: cName, role: "content_operator", brand_id: cBrand || undefined, channel: cChannel,
        auto_publish: false,
        identity: { name: cName, role: "content_operator", voice: "neutral", language: "zh-CN", audience: [], expertise: [], values: [], boundaries: [] },
        mission: { statement: "运营账号建立领域权威", domain: "content", audience: [], value_proposition: "", constraints: [], success_definition: "互动与粉丝增长" },
        strategy: { pillars: [], cadence_per_week: 7, formats: ["text"], risk_tolerance: 0.3, exploration_budget: 0.2 },
        focus: {
          scope: cScope,
          industry: cScope === "niche" ? cIndustry : undefined,
          sub_verticals: cVerticals.split(",").map((x) => x.trim()).filter(Boolean),
          topics: cTopics.split(",").map((x) => x.trim()).filter(Boolean),
          audience: [],
          excluded_topics: cExcluded.split(",").map((x) => x.trim()).filter(Boolean),
        },
      });
      setSaved("员工已创建 ✅");
      setCreating(false); setCName("");
      const d = await api.listEmployees(); setEmps(d.items); setSel(r.id);
    } catch (e) { setSaved(`创建失败: ${e}`); }
  };

  const saveEdit = async () => {
    if (!sel) return;
    setSaved(null);
    try {
      let focus: unknown = undefined;
      if (eFocus.trim()) focus = JSON.parse(eFocus);
      await api.patchEmployee(sel, {
        name: eName, role: eRole, channel: eChannel,
        focus,
        identity: eVoice.trim() ? { voice: eVoice } : undefined,
        mission: eMission.trim() ? { statement: eMission } : undefined,
      });
      setSaved("员工已更新 ✅");
      setEditing(false);
      const d = await api.listEmployees(); setEmps(d.items);
    } catch (e) { setSaved(`保存失败: ${e}`); }
  };
  const removeEmp = async () => {
    if (!sel) return;
    if (!window.confirm("确认删除该员工? (软归档为 ARCHIVED, 数据保留, 可手工恢复; RUNNING 状态需先停止)")) return;
    try {
      await api.deleteEmployee(sel);
      const d = await api.listEmployees(); setEmps(d.items); setSel(d.items[0]?.id ?? null);
    } catch (e) { setSaved(`删除失败: ${e}`); }
  };

  return (
    <section className="view active">
      <h1>🧑‍🚀 数字员工</h1>
      <p className="lede">业务中心之一。每个员工按「观察→理解→决策→计划→创作→审核→执行→衡量→学习→适应」10 步自主运营一个账号，目标是涨粉。</p>

      <div className="grid g3">
        <div className="card">
          <h3>员工列表 <span className="sp" />
            <button className="btn emp sm" onClick={() => setCreating((v) => !v)}>➕ 新建员工</button>
          </h3>
          {creating && (
            <div style={{ border: "1px solid var(--line2)", borderRadius: 10, padding: 10, marginBottom: 8 }}>
              <div className="row" style={{ gap: 6, flexWrap: "wrap" }}>
                <input placeholder="姓名" value={cName} onChange={(e) => setCName(e.target.value)} style={{ width: 110 }} />
                <select value={cBrand} onChange={(e) => setCBrand(e.target.value)}>
                  {brands.map((b) => <option key={b.id} value={b.id}>{b.name}</option>)}
                </select>
                <input placeholder="渠道" value={cChannel} onChange={(e) => setCChannel(e.target.value)} style={{ width: 60 }} />
                <select value={cScope} onChange={(e) => setCScope(e.target.value)}>
                  <option value="niche">细分领域</option>
                  <option value="general">不分领域</option>
                </select>
                {cScope === "niche" && (
                  <select value={cIndustry} onChange={(e) => setCIndustry(e.target.value)}>
                    <option value="">选行业 *</option>
                    {industries.map((x) => <option key={x.id} value={x.id}>{x.name} ({x.id})</option>)}
                  </select>
                )}
              </div>
              {cScope === "niche" && (
                <div className="row" style={{ gap: 6, flexWrap: "wrap", marginTop: 6 }}>
                  <input placeholder="赛道 (逗号分隔)" value={cVerticals} onChange={(e) => setCVerticals(e.target.value)} style={{ width: 170 }} />
                  <input placeholder="话题 (逗号分隔)" value={cTopics} onChange={(e) => setCTopics(e.target.value)} style={{ width: 150 }} />
                  <input placeholder="排除话题" value={cExcluded} onChange={(e) => setCExcluded(e.target.value)} style={{ width: 120 }} />
                </div>
              )}
              <div style={{ marginTop: 8 }}>
                <button className="btn emp sm" disabled={!cName || (cScope === "niche" && !cIndustry)} onClick={createEmp}>创建</button>
                <button className="btn sm" style={{ marginLeft: 6 }} onClick={() => setCreating(false)}>取消</button>
                {saved && <span className="sub" style={{ marginLeft: 8 }}>{saved}</span>}
              </div>
            </div>
          )}
          {emps.map((e) => (
            <div
              key={e.id}
              onClick={() => setSel(e.id)}
              style={{
                border: `1px solid ${e.id === sel ? "var(--emp)" : "var(--line2)"}`,
                background: e.id === sel ? "var(--emp-bg)" : "transparent",
                borderRadius: 10, padding: 10, marginBottom: 8, cursor: "pointer",
              }}
            >
              <div className="row">
                <b>{e.name}</b>
                <Badge kind={e.status === "RUNNING" ? "run" : "pend"}>{e.status}</Badge>
                <span className="sp" /><span className="sub">{e.channel}</span>
              </div>
              <div className="sub" style={{ marginTop: 4 }}>{e.role} · {e.auto_publish ? "自动发布" : "进待审队列"}</div>
            </div>
          ))}
          {emps.length === 0 && <p className="sub">还没有员工，先在设置里创建品牌与员工。</p>}
        </div>

        <div className="card" style={{ gridColumn: "span 2" }}>
          <h3>{emp?.name ?? "—"} · 本周期
            <span className="sp" />
            <button className="btn sm" onClick={() => setEditing((v) => !v)}>✏️ 编辑</button>
            <button className="btn sm danger" style={{ marginLeft: 6 }} onClick={removeEmp}>🗑 删除</button>
            <span className="sub">{cycles?.[0] ? `cycle ${cycles[0].id.slice(0, 8)} · ${cycles[0].trigger} · ${cycles[0].created_at.slice(0, 16).replace("T", " ")}` : ""}</span>
          </h3>
          <div className="cycle mt14">
            {STEPS.map((s, i) => {
              const t = detail?.tasks.find((x) => x.step === s);
              const cls = t ? (t.state === "SUCCEEDED" || t.state === "DONE" ? "done" : "now") : "";
              return <div key={s} className={`cstep ${cls}`}>{i + 1} {STEP_ZH[s]}</div>;
            })}
          </div>
          <div className="sub mt14">
            {detail?.summary ? <>决策: {String((detail.summary as Record<string, unknown>).selected_opportunity ?? "—")}</> : "暂无周期数据"}
          </div>
        </div>
      </div>

      {editing && emp && (
        <div className="card mt14">
          <h3>✏️ 编辑员工 {emp.name}</h3>
          <div className="grid g3">
            <div className="field"><div><label>姓名</label></div><input value={eName} onChange={(e) => setEName(e.target.value)} /></div>
            <div className="field"><div><label>角色 role</label></div><input value={eRole} onChange={(e) => setERole(e.target.value)} /></div>
            <div className="field"><div><label>渠道 channel</label></div><input value={eChannel} onChange={(e) => setEChannel(e.target.value)} style={{ width: 80 }} /></div>
            <div className="field"><div><label>Voice 单字段</label></div><input value={eVoice} onChange={(e) => setEVoice(e.target.value)} placeholder="neutral" /></div>
            <div className="field"><div><label>使命 statement</label></div><input value={eMission} onChange={(e) => setEMission(e.target.value)} style={{ width: 260 }} /></div>
          </div>
          <div className="field"><div><label>领域画像 focus (JSON)</label><div className="hint">改完自动校验; niche 必须带合法 industry</div></div></div>
          <textarea rows={6} value={eFocus} onChange={(e) => setEFocus(e.target.value)} />
          <div style={{ marginTop: 8 }}>
            <button className="btn emp sm" onClick={saveEdit}>保存修改</button>
            <button className="btn sm" style={{ marginLeft: 8 }} onClick={() => setEditing(false)}>取消</button>
            {saved && <span className="sub" style={{ marginLeft: 8 }}>{saved}</span>} 
          </div>
        </div>
      )}

      <div className="grid g3 mt14">
        <div className="card">
          <h3>⚙️ 策略配置 <span className="sub">(改完保存, 落库为新版本)</span></h3>
          <div className="field"><div><label>目标 KPI · 粉丝</label><div className="hint">涨粉是运营目标</div></div>
            <input type="number" value={fTarget} onChange={(e) => setFTarget(e.target.value)} style={{ width: 90 }} placeholder="如 10000" /></div>
          <div className="field"><div><label>发布节奏 / 周</label></div>
            <input type="number" value={fCadence} onChange={(e) => setFCadence(e.target.value)} style={{ width: 60 }} /></div>
          <div className="field"><div><label>自动发布</label><div className="hint">关 = 进待审队列</div></div>
            <div className={`toggle${fAuto ? " on" : ""}`} onClick={() => setFAuto(!fAuto)} /></div>
          <div className="field"><div><label>探索预算</label><div className="hint">试新角度的比例</div></div>
            <input type="number" step="0.1" value={fExplore} onChange={(e) => setFExplore(e.target.value)} style={{ width: 60 }} /></div>
          <div className="field mb0"><div><label>内容支柱 pillars</label><div className="hint">逗号分隔; 空 = 不做相关性过滤</div></div>
            <input type="text" value={fPillars} onChange={(e) => setFPillars(e.target.value)} style={{ width: 140 }} placeholder="ai agents, llm ops" /></div>
          <div style={{ marginTop: 10 }}>
            <button className="btn emp sm" onClick={saveStrategy}>保存策略</button>
            {saved && <span className="sub" style={{ marginLeft: 8 }}>{saved}</span>}
          </div>
        </div>

        <div className="card">
          <h3>📈 粉丝轨迹 <span className="sub">{myMetric ? `${myMetric.platform} · @${myMetric.account_id}` : "未绑定账号"}</span></h3>
          {myMetric ? (
            <>
              <div className="kpi">{myMetric.followers ?? "—"} <span className="sub" style={{ fontSize: 13 }}>粉丝数</span> {fmtDelta(myMetric.delta_7d)}</div>
              <Spark points={myMetric.series.map((s) => s.followers)} />
              <div className="sub">
                {target ? `目标 ${target.toLocaleString()} · 进度 ${progress ?? "—"}% · 7 日 Δ ${myMetric.delta_7d ?? "—"}` : "未设目标 KPI — 在左侧策略里配置 target_followers"}
              </div>
              {progress !== null && (
                <div style={{ marginTop: 6, height: 8, borderRadius: 4, background: "var(--panel2)" }}>
                  <div style={{ width: `${progress}%`, height: "100%", borderRadius: 4, background: "var(--emp)" }} />
                </div>
              )}
            </>
          ) : (
            <>
              <div className="kpi">—</div>
              <Spark points={[]} dashed />
              <div className="sub">read_followers 设备任务已启用 (adapter active), 设备接入后自动点亮; 也可 POST /intel/account-metrics 灌入 API 样本</div>
            </>
          )}
        </div>

        <div className="card">
          <h3>🧠 学习 & 实验 <span className="sub">(断点②已闭合)</span></h3>
          {!insights && <Loading />}
          {insights?.map((i) => (
            <div key={i.id} style={{ fontSize: 12, borderLeft: "2px solid var(--emp)", paddingLeft: 10, marginBottom: 8 }}>
              <b>insight</b>: {i.content.slice(0, 80)}<br /><span className="sub">conf {i.confidence} · {i.created_at.slice(0, 16).replace("T", " ")}</span>
            </div>
          ))}
          {insights?.length === 0 && <p className="sub">还没有学习结论 — 周期跑起来后由 Learn/Adapt 写入。</p>}
        </div>
      </div>
      {sel && <IntelBoardPanel empId={sel} focus={emp?.focus} />}    </section>
  );
}

// WS3: 领域情报仪表板 + 专属 feed (按 focus 过滤; general = 全量)
function IntelBoardPanel({ empId, focus }: { empId: string; focus?: Record<string, unknown> }) {
  const [win, setWin] = useState("24h");
  const [board, setBoard] = useState<IntelBoard | null>(null);
  const [feed, setFeed] = useState<{ signals: unknown[]; opportunities: unknown[]; filtered: boolean } | null>(null);
  const [err, setErr] = useState("");
  useEffect(() => {
    api.getIntelBoard(empId, win).then(setBoard).catch((e) => setErr(String(e)));
    api.getEmployeeFeed(empId).then(setFeed).catch(() => {});
  }, [empId, win]);
  const f = focus ?? {};
  const scope = (f.scope as string) ?? "general";
  return (
    <div className="mt14">
      <div className="card">
        <h3>📡 情报板 <span className="sub">{scope === "niche" ? `细分领域: ${String(f.industry ?? "—")} (已按领域过滤)` : "不分领域 (全量)"}</span>
          <span className="sp" />
                    {["8h", "16h", "24h"].map((w) => (
            <button key={w} className={`btn sm ${win === w ? "emp" : ""}`} style={{ marginLeft: 6 }} onClick={() => setWin(w)}>{w}</button>
          ))}
        </h3>
        {err && <Err e={err} />}
        {!board && <Loading />}    
        {board && (
          <div className="grid g2">
            <div>
              <h4>🔥 热帖/趋势 ({board.window})</h4>
              {board.hot.slice(0, 8).map((h) => (
                <div key={h.signal_id} style={{ fontSize: 12, marginBottom: 6 }}>
                  <Badge kind="run">{h.signal_type}</Badge> {h.title ?? h.signal_id} <span className="sub">imp {h.importance.toFixed(2)}</span>
                </div>
              ))}
              {board.hot.length === 0 && <p className="sub">窗口内本领域暂无信号</p>}
            </div>
            <div>
              <h4>🚀 爆款话题 / 竞品动态</h4>
              {board.viral.slice(0, 8).map((h) => (
                <div key={h.signal_id} style={{ fontSize: 12, marginBottom: 6 }}>
                  <Badge kind="warn">{h.signal_type}</Badge> {h.title ?? h.signal_id}
                </div>
              ))}
              {board.viral.length === 0 && <p className="sub">窗口内无爆款/竞品信号</p>}
            </div>
            <div>
              <h4>🌟 本领域 Top KOL</h4>
              {board.kols.status !== "ok" && <p className="sub">{board.kols.note ?? "未配置"}</p>}
              {board.kols.items.slice(0, 10).map((k) => (
                <div key={k.entity_id} style={{ fontSize: 12, marginBottom: 6 }}>{k.title || k.entity_id} <span className="sub">{(k.industry_tags ?? []).join(",")}</span></div>
              ))}
            </div>
            <div>
              <h4>💡 选题缺口 (领域热点 ⊖ 已发)</h4>
              {board.topic_gaps.slice(0, 8).map((h) => (
                <div key={h.signal_id} style={{ fontSize: 12, marginBottom: 6 }}>{h.title ?? h.signal_id}</div>
              ))}
              {board.topic_gaps.length === 0 && <p className="sub">暂无缺口 (或近期已覆盖)</p>}
            </div>
          </div>
        )}
      </div>
      {feed && (
        <div className="card mt14">
          <h3>📥 专属 feed <span className="sub">{feed.filtered ? "已按领域过滤" : "全量"}</span></h3>
          <div className="grid g2">
            <div>
              <h4>信号 ({feed.signals.length})</h4>
              {feed.signals.slice(0, 8).map((x) => { const s = x as { signal_id: string; signal_type: string; importance: number; payload?: { title?: string } };
                return <div key={s.signal_id} style={{ fontSize: 12, marginBottom: 6 }}><Badge kind="run">{s.signal_type}</Badge> {s.payload?.title ?? s.signal_id}</div>; })}
            </div>
            <div>
              <h4>机会 ({feed.opportunities.length})</h4>
              {feed.opportunities.slice(0, 8).map((x) => { const o = x as { topic: string; score: number };
                return <div key={o.topic} style={{ fontSize: 12, marginBottom: 6 }}>{o.topic} <span className="sub">score {typeof o.score === "number" ? o.score.toFixed(2) : String(o.score)}</span></div>; })}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
