"use client";

// 🎯 KPI 配额 (claude/kpi-schema.md): 三档视图
//   1) 分类档位 6 档 (category 默认)
//   2) 平台画像 N 个 (模板, 可点开看完整 A/B/C/D)
//   3) 账号 KPI: 选员工+平台+账号 → 模板⊕覆盖编辑 → 保存落库; 今日 ledger 进度条

import { useEffect, useMemo, useState } from "react";

import {
  api,
  type AccountKpi,
  type EmployeeItem,
  type KpiCategory,
  type KpiEffective,
  type KpiPlatform,
  type LedgerToday,
} from "@/lib/api";
import { Badge, Err, Loading } from "@/components/ui";

const QUOTA_LABEL: Record<string, string> = {
  posts_per_day: "发帖/天", posts_per_week: "发帖/周", replies_per_day: "回复/天", comments_per_day: "评论/天",
  follows_per_day: "关注/天", likes_per_day: "点赞/天", reposts_per_day: "转发/天", quotes_per_day: "引用/天",
  dms_per_day: "私信/天", saves_per_day: "收藏/天", duet_stitch_per_day: "二创/天", connects_per_day: "邀请/天",
  reacts_per_day: "反应/天", upvotes_per_day: "upvote/天", repins_per_day: "repin/天", community_posts_per_day: "社区帖/天",
  dynamics_per_day: "动态/天", reels_per_day: "Reels/天", stories_per_day: "故事/天", crossposts_per_day: "crosspost/天",
  boosts_per_day: "boost/天",
};

function QuotaTable({ quotas }: { quotas: Record<string, number | string[] | null> }) {
  const entries = Object.entries(quotas).filter(([, v]) => typeof v === "number" || v === null);
  return (
    <table><tr><th>动作配额</th><th>值</th><th>语义</th></tr>
      {entries.map(([k, v]) => (
        <tr key={k}>
          <td>{QUOTA_LABEL[k] ?? k}</td>
          <td><b>{v === null ? "—" : v}</b></td>
          <td className="sub">{v === 0 ? "禁用" : v === null ? "该平台无此动作" : "必达(受健康压顶)"}</td>
        </tr>
      ))}
      {entries.length === 0 && <tr><td colSpan={3} className="sub">无</td></tr>}
    </table>
  );
}

export function KpiView() {
  const [seg, setSeg] = useState<"categories" | "platforms" | "accounts">("platforms");
  const [cats, setCats] = useState<KpiCategory[] | null>(null);
  const [plats, setPlats] = useState<KpiPlatform[] | null>(null);
  const [accounts, setAccounts] = useState<AccountKpi[] | null>(null);
  const [emps, setEmps] = useState<EmployeeItem[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);

  // 平台详情
  const [platDetail, setPlatDetail] = useState<KpiEffective | null>(null);
  // 账号编辑器状态
  const [selEmp, setSelEmp] = useState<string>("");
  const [selPlat, setSelPlat] = useState<string>("x");
  const [accountId, setAccountId] = useState<string>("");
  const [eff, setEff] = useState<KpiEffective | null>(null);
  const [overrides, setOverrides] = useState<Record<string, unknown>>({});
  const [ledger, setLedger] = useState<LedgerToday | null>(null);

  useEffect(() => {
    Promise.all([api.listKpiCategories(), api.listKpiPlatforms(), api.listKpiAccounts(), api.listEmployees()])
      .then(([c, p, a, e]) => {
        setCats(c.items); setPlats(p.items); setAccounts(a.items); setEmps(e.items);
        if (e.items[0]) setSelEmp(e.items[0].id);
      })
      .catch((e) => setErr(String(e)));
  }, []);

  // 选中账号 → 拉 effective + ledger
  useEffect(() => {
    if (!selPlat || !accountId.trim()) { setEff(null); setLedger(null); return; }
    api.getKpiEffective(selPlat, accountId.trim(), selEmp || undefined)
      .then((d) => { setEff(d.effective); setOverrides(d.overrides ?? {}); })
      .catch((e) => setErr(String(e)));
    api.getKpiLedgerToday(selPlat, accountId.trim()).then(setLedger).catch(() => setLedger(null));
  }, [selPlat, accountId, selEmp]);

  const catOf = useMemo(() => Object.fromEntries((cats ?? []).map((c) => [c.category, c])), [cats]);

  const saveOverrides = async () => {
    setMsg(null);
    try {
      await api.putKpiAccount({
        employee_id: selEmp || undefined,
        platform: selPlat,
        account_id: accountId.trim(),
        overrides,
      });
      setMsg("账号 KPI 覆盖已保存, effective 已重算 ✅");
      api.listKpiAccounts().then((d) => setAccounts(d.items)).catch(() => {});
    } catch (e) { setMsg(`保存失败: ${e}`); }
  };

  // 编辑某个 quota 字段 (写进 overrides.activity_quotas)
  const setQuota = (key: string, val: string) => {
    const n = val.trim() === "" ? null : Number(val);
    setOverrides((o) => ({
      ...o,
      activity_quotas: { ...((o.activity_quotas as Record<string, unknown>) ?? {}), [key]: Number.isNaN(n as number) ? null : n },
    }));
  };
  const setGrowth = (key: string, val: string) => {
    const n = Number(val);
    setOverrides((o) => ({
      ...o,
      growth_targets: { ...((o.growth_targets as Record<string, unknown>) ?? {}), [key]: val.trim() === "" ? null : (Number.isNaN(n) ? val : n) },
    }));
  };
  const setGuard = (key: string, val: string) => {
    const n = Number(val);
    setOverrides((o) => ({
      ...o,
      health_guardrails: { ...((o.health_guardrails as Record<string, unknown>) ?? {}), [key]: val.trim() === "" ? null : (Number.isNaN(n) ? val : n) },
    }));
  };

  if (err) return <Err e={err} />;
  if (!cats || !plats || !accounts || !emps) return <Loading />;

  return (
    <section className="view active">
      <h1>🎯 KPI / 配额 (全平台)</h1>
      <p className="lede">
        两类分开: <b>A 活动配额</b>(你下、系统必达、受 D 压顶) vs <b>C 增长目标</b>(你设、尽力不保证)。
        两级生效: 平台模板 ⊕ 账号覆盖 = 本账号 effective。优先级: 健康 D &gt; 合规 B &gt; 配额 A &gt; 增长加力 C。
      </p>

      <div className="seg" style={{ marginBottom: 14 }}>
        <button className={seg === "categories" ? "on" : ""} onClick={() => setSeg("categories")}>① 分类档位 ({cats.length})</button>
        <button className={seg === "platforms" ? "on" : ""} onClick={() => setSeg("platforms")}>② 平台画像 ({plats.length})</button>
        <button className={seg === "accounts" ? "on" : ""} onClick={() => setSeg("accounts")}>③ 账号 KPI ({accounts.length})</button>
      </div>

      {seg === "categories" && (
        <div className="grid g2">
          {cats.map((c) => (
            <div className="card" key={c.category}>
              <h3>{c.label} <span className="tag t-emp">{c.category}</span></h3>
              <div className="sub">增长指标: {c.growth_metric} · 互动: {c.engagement_formula}</div>
              <div className="sub">动作: {c.available_actions.join(", ")}</div>
              <div className="mt14"><QuotaTable quotas={c.default_quotas} /></div>
            </div>
          ))}
        </div>
      )}

      {seg === "platforms" && (
        <>
          <div className="card">
            <h3>平台画像 <span className="sub">点行看完整 A/B/C/D 模板</span></h3>
            <table><tr><th>平台</th><th>档位</th><th>增长指标</th><th>发帖</th><th>回复</th><th>关注</th><th>点赞</th><th>日上限</th><th>健康信号</th></tr>
              {plats.map((p) => {
                const q = p.default_quotas;
                const cap = (p.rate_caps?.daily_action_ceiling as number) ?? null;
                return (
                  <tr key={p.platform} style={{ cursor: "pointer" }} onClick={() => api.getKpiPlatform(p.platform).then(setPlatDetail).catch((e) => setErr(String(e)))}>
                    <td><b>{p.label}</b> <span className="sub">{p.platform}</span></td>
                    <td><Badge kind="pend">{catOf[p.category]?.label ?? p.category}</Badge></td>
                    <td>{p.growth_metric}</td>
                    <td>{q.posts_per_day ?? (q.posts_per_week != null ? `${q.posts_per_week}/周` : "—")}</td>
                    <td>{q.replies_per_day ?? q.comments_per_day ?? "—"}</td>
                    <td>{q.follows_per_day ?? "—"}</td>
                    <td>{q.likes_per_day ?? "—"}</td>
                    <td>{cap ?? "—"}</td>
                    <td className="sub">{p.health_signals.slice(0, 3).join(", ")}</td>
                  </tr>
                );
              })}
            </table>
          </div>
          {platDetail && (
            <div className="card mt14">
              <h3>{platDetail.platform} · 完整模板 <span className="sp" /><button className="btn ghost sm" onClick={() => setPlatDetail(null)}>关闭</button></h3>
              <div className="grid g2">
                <div><h3 className="sub">A 活动配额</h3><QuotaTable quotas={platDetail.activity_quotas} /></div>
                <div>
                  <h3 className="sub">B 质量门</h3>
                  <pre className="sub" style={{ whiteSpace: "pre-wrap" }}>{JSON.stringify(platDetail.quality_gates, null, 1)}</pre>
                  <h3 className="sub mt14">D 健康红线</h3>
                  <pre className="sub" style={{ whiteSpace: "pre-wrap" }}>{JSON.stringify(platDetail.health_guardrails, null, 1)}</pre>
                </div>
              </div>
              <div className="sub mt14">二级指标: {platDetail.secondary_metrics.join(", ")} · 需要的适配器: {platDetail.available_actions.join(", ")} + read_followers/read_mentions</div>
            </div>
          )}
        </>
      )}

      {seg === "accounts" && (
        <div className="grid g2">
          <div className="card">
            <h3>选择账号</h3>
            <div className="row" style={{ flexWrap: "wrap" }}>
              <select value={selEmp} onChange={(e) => setSelEmp(e.target.value)}>
                {emps.map((e) => <option key={e.id} value={e.id}>{e.name} ({e.channel})</option>)}
              </select>
              <select value={selPlat} onChange={(e) => setSelPlat(e.target.value)}>
                {plats.map((p) => <option key={p.platform} value={p.platform}>{p.label}</option>)}
              </select>
              <input type="text" placeholder="account_id (如 demo_account)" value={accountId} onChange={(e) => setAccountId(e.target.value)} style={{ width: 200 }} />
            </div>
            <div className="sub mt14">已有账号覆盖:</div>
            <table><tr><th>平台</th><th>账号</th><th>员工</th><th>更新</th></tr>
              {accounts.map((a) => (
                <tr key={`${a.platform}:${a.account_id}`} style={{ cursor: "pointer" }}
                  onClick={() => { setSelPlat(a.platform); setAccountId(a.account_id); if (a.employee_id) setSelEmp(a.employee_id); }}>
                  <td>{a.platform}</td><td>{a.account_id}</td>
                  <td className="sub">{emps.find((e) => e.id === a.employee_id)?.name ?? "—"}</td>
                  <td className="sub">{a.updated_at.slice(5, 16).replace("T", " ")}</td>
                </tr>
              ))}
              {accounts.length === 0 && <tr><td colSpan={4} className="sub">还没有账号覆盖 — 选平台+账号后在右侧编辑保存即创建</td></tr>}
            </table>
            {ledger && (
              <div className="mt14">
                <h3>今日消耗 (ledger) {ledger.ceiling_blocked && <Badge kind="bad">日上限已到顶 🛑</Badge>}</h3>
                {Object.entries(ledger.actions).map(([k, a]) => (
                  a.quota === null || a.not_applicable ? null : (
                    <div key={k} className="field">
                      <div><label>{QUOTA_LABEL[k] ?? k}</label>
                        <div className="hint">{a.disabled ? "已禁用" : `已用 ${a.used} / 配额 ${a.quota}`}</div></div>
                      <div style={{ width: 140, height: 8, borderRadius: 4, background: "var(--panel2)" }}>
                        <div style={{
                          width: `${Math.min(100, a.quota ? (a.used / a.quota) * 100 : 0)}%`, height: "100%",
                          borderRadius: 4, background: a.remaining === 0 ? "var(--warn)" : "var(--ok)",
                        }} />
                      </div>
                    </div>
                  )
                ))}
                <div className="sub">日总动作 {ledger.daily_total} / 上限 {ledger.daily_ceiling ?? "—"}</div>
              </div>
            )}
          </div>

          <div className="card">
            <h3>账号 KPI 覆盖编辑 <span className="sub">{selPlat} · {accountId || "(填 account_id)"}</span></h3>
            {!eff && <Loading text="填写 account_id 后加载 effective…" />}
            {eff && (
              <>
                <div className="sub">模板档位: {catOf[eff.category]?.label ?? eff.category} · 增长指标 {eff.growth_metric} · 互动 {eff.engagement_formula}</div>
                <h3 className="mt14">A 活动配额覆盖 <span className="sub">(空白 = 用模板默认; 0=禁用; 留空并保存 null=平台无)</span></h3>
                <table><tr><th>动作</th><th>模板值</th><th>本账号覆盖</th></tr>
                  {Object.entries(eff.activity_quotas).filter(([, v]) => typeof v === "number" || v === null).map(([k, v]) => {
                    const ov = (overrides.activity_quotas as Record<string, unknown> | undefined)?.[k];
                    return (
                      <tr key={k}>
                        <td>{QUOTA_LABEL[k] ?? k}</td>
                        <td className="sub">{v === null ? "—" : v}</td>
                        <td><input type="number" style={{ width: 80 }} placeholder={String(v ?? "")}
                          value={ov === undefined ? "" : String(ov)}
                          onChange={(e) => setQuota(k, e.target.value)} /></td>
                      </tr>
                    );
                  })}
                </table>
                <h3 className="mt14">C 增长目标 <span className="sub">(尽力, 不保证)</span></h3>
                <div className="row" style={{ flexWrap: "wrap" }}>
                  <label className="sub">目标值 <input type="number" style={{ width: 100, marginLeft: 6 }}
                    placeholder={String(eff.growth_targets?.target ?? "")}
                    value={String((overrides.growth_targets as Record<string, unknown> | undefined)?.target ?? "")}
                    onChange={(e) => setGrowth("target", e.target.value)} /></label>
                  <label className="sub">周期天数 <input type="number" style={{ width: 80, marginLeft: 6 }}
                    placeholder={String(eff.growth_targets?.horizon_days ?? 90)}
                    value={String((overrides.growth_targets as Record<string, unknown> | undefined)?.horizon_days ?? "")}
                    onChange={(e) => setGrowth("horizon_days", e.target.value)} /></label>
                  <label className="sub">日净增 <input type="number" style={{ width: 80, marginLeft: 6 }}
                    placeholder={String(eff.growth_targets?.daily_net_growth_min ?? "")}
                    value={String((overrides.growth_targets as Record<string, unknown> | undefined)?.daily_net_growth_min ?? "")}
                    onChange={(e) => setGrowth("daily_net_growth_min", e.target.value)} /></label>
                </div>
                <h3 className="mt14">D 健康红线 <span className="sub">(压顶 A; 命中信号自动停)</span></h3>
                <div className="row" style={{ flexWrap: "wrap" }}>
                  <label className="sub">每小时限速 <input type="number" style={{ width: 70, marginLeft: 6 }}
                    placeholder={String(eff.health_guardrails?.max_actions_per_hour ?? "")}
                    value={String((overrides.health_guardrails as Record<string, unknown> | undefined)?.max_actions_per_hour ?? "")}
                    onChange={(e) => setGuard("max_actions_per_hour", e.target.value)} /></label>
                  <label className="sub">日总上限 <input type="number" style={{ width: 70, marginLeft: 6 }}
                    placeholder={String(eff.health_guardrails?.daily_action_ceiling ?? "")}
                    value={String((overrides.health_guardrails as Record<string, unknown> | undefined)?.daily_action_ceiling ?? "")}
                    onChange={(e) => setGuard("daily_action_ceiling", e.target.value)} /></label>
                  <label className="sub">冷却分钟 <input type="number" style={{ width: 70, marginLeft: 6 }}
                    placeholder={String(eff.health_guardrails?.cooldown_minutes_after_block ?? "")}
                    value={String((overrides.health_guardrails as Record<string, unknown> | undefined)?.cooldown_minutes_after_block ?? "")}
                    onChange={(e) => setGuard("cooldown_minutes_after_block", e.target.value)} /></label>
                </div>
                <div style={{ marginTop: 12 }}>
                  <button className="btn emp sm" onClick={saveOverrides} disabled={!accountId.trim()}>保存账号覆盖</button>
                  {msg && <span className="sub" style={{ marginLeft: 8 }}>{msg}</span>}
                </div>
                <div className="sub mt14">B 质量门沿用模板 (min_quality 60 / 事实核验 / 查重 0.2 / 禁用词 / AI 标识), 工序5 已硬拦截。</div>
              </>
            )}
          </div>
        </div>
      )}
    </section>
  );
}
