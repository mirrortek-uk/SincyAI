"use client";

// 🏭 内容工厂: KPI + 工序 0-9 流水线看板 + 待审批队列 + 触发入口(建 Brief) + 工序9 学习

import { useEffect, useState } from "react";

import { api, type BrandItem, type FactoryBrief, type PackageItem } from "@/lib/api";
import { goView } from "@/components/Shell";
import { Badge, Err, Loading } from "@/components/ui";

const COLS: { key: string; label: string; match: (b: FactoryBrief) => boolean }[] = [
  { key: "draft", label: "0 Brief", match: (b) => b.status === "draft" },
  { key: "running", label: "1-7 加工", match: (b) => b.status === "running" },
  { key: "done", label: "8 发布", match: (b) => b.status === "done" },
  { key: "failed", label: "被门拦下", match: (b) => b.status === "failed" },
];

export function FactoryView({ openNew }: { openNew: boolean }) {
  const [briefs, setBriefs] = useState<FactoryBrief[] | null>(null);
  const [pkgs, setPkgs] = useState<PackageItem[] | null>(null);
  const [brands, setBrands] = useState<BrandItem[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [showNew, setShowNew] = useState(false);
  const [goal, setGoal] = useState("");
  const [brandId, setBrandId] = useState("");
  const [platforms, setPlatforms] = useState("x");

  const load = () => {
    setErr(null);
    Promise.all([api.listFactoryBriefs(), api.listPackages(), api.listBrands()])
      .then(([b, p, br]) => {
        setBriefs(b.briefs); setPkgs(p.items); setBrands(br.items);
        if (br.items[0] && !brandId) setBrandId(br.items[0].id);
      })
      .catch((e) => setErr(String(e)));
  };
  useEffect(() => { load(); }, []); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { if (openNew) setShowNew(true); }, [openNew]);

  if (err) return <Err e={err} />;
  if (!briefs || !pkgs || !brands) return <Loading />;

  const inflight = briefs.filter((b) => b.status === "draft" || b.status === "running").length;
  const pending = pkgs.filter((p) => p.approval_status === "pending");
  const today = new Date().toISOString().slice(0, 10);
  const publishedToday = pkgs.filter((p) => p.approval_status !== "pending" && p.approval_status !== "rejected" && p.created_at.slice(0, 10) === today).length;
  const blocked = briefs.filter((b) => b.status === "failed").length;

  const create = async () => {
    setMsg(null);
    try {
      await api.createFactoryBrief({
        trigger: "manual",
        brand_id: brandId || (brands[0]?.id ?? ""),
        topic: goal,
        goal,
        platforms: platforms.split(",").map((s) => s.trim()).filter(Boolean),
      });
      setMsg("Brief 已创建 (draft), 后台 runner 自动消费 ✅");
      setShowNew(false); setGoal(""); load();
    } catch (e) { setMsg(`创建失败: ${e}`); }
  };

  return (
    <section className="view active">
      <h1>🏭 内容工厂</h1>
      <p className="lede">业务中心之二。一条内容从 Brief 到发布的流水线，三种入口触发，工序 0→7 逐站加工，质量门 + 审批后打包发布，工序9 学习回流。</p>

      <div className="grid g4">
        <div className="card"><div className="sub">在流转</div><div className="kpi" style={{ color: "var(--fac)" }}>{inflight}</div></div>
        <div className="card"><div className="sub">待审批</div><div className="kpi">{pending.length}</div></div>
        <div className="card"><div className="sub">今日发布</div><div className="kpi">{publishedToday}</div></div>
        <div className="card"><div className="sub">被门拦下</div><div className="kpi" style={{ color: "var(--bad)" }}>{blocked}</div></div>
      </div>

      <div className="card mt14">
        <h3>触发入口 → 工序0 Brief <span className="sp" />
          <button className="btn ghost sm" onClick={() => goView("signals")}>① 机会驱动</button>
          <button className="btn ghost sm" onClick={() => setMsg("计划驱动: 内容日历到点自动建 Brief (calendar runner)")}>② 计划驱动</button>
          <button className="btn fac sm" onClick={() => setShowNew(!showNew)}>③ 人工建 Brief</button>
        </h3>
        {showNew && (
          <div className="row mt14" style={{ flexWrap: "wrap" }}>
            <select value={brandId} onChange={(e) => setBrandId(e.target.value)}>
              {brands.map((b) => <option key={b.id} value={b.id}>{b.name}</option>)}
            </select>
            <input type="text" placeholder="选题 / goal" value={goal} onChange={(e) => setGoal(e.target.value)} style={{ flex: 1, minWidth: 200 }} />
            <input type="text" placeholder="平台, 逗号分隔 (x,youtube…)" value={platforms} onChange={(e) => setPlatforms(e.target.value)} style={{ width: 180 }} />
            <button className="btn fac sm" onClick={create} disabled={!goal.trim()}>创建 Brief</button>
          </div>
        )}
        {msg && <p className="sub mt14">{msg}</p>}
        <div className="line mt14">
          {COLS.map((c) => {
            const items = briefs.filter(c.match);
            return (
              <div className="col" key={c.key}>
                <div className="ch" style={c.key === "failed" ? { color: "var(--bad)" } : undefined}>{c.label}<span>{items.length}</span></div>
                {items.slice(0, 8).map((b) => (
                  <div className="item" key={b.brief_id} onClick={() => goView("lineage")}>
                    <div className="tp">{b.goal || b.angle || b.brief_id.slice(0, 12)}</div>
                    <div className="mt">{b.trigger} · {(b.platforms ?? []).join(",") || "—"} · {b.created_at.slice(5, 16).replace("T", " ")}</div>
                  </div>
                ))}
                {items.length === 0 && <div className="sub" style={{ fontSize: 10 }}>空</div>}
              </div>
            );
          })}
        </div>
        <div className="sub mt14">点任一卡片 → 内容流转追踪 (lineage)。Brief 状态: draft(0) → running(1-7) → done(8) / failed(被拦)。</div>
      </div>

      <div className="grid g2 mt14">
        <div className="card">
          <h3>✅ 待审批队列 <span className="tag t-fac">工序6</span> <span className="sp" />
            <button className="btn ghost sm" onClick={() => goView("approvals")}>全部 →</button></h3>
          <table><tr><th>内容</th><th>平台</th><th>来源</th><th>操作</th></tr>
            {pending.slice(0, 10).map((p) => (
              <tr key={p.id}>
                <td>{p.text_preview.slice(0, 40)}</td>
                <td>{p.platform}</td>
                <td><Badge kind="pend">{p.constraints?.source ?? "—"}</Badge></td>
                <td>
                  <button className="btn sm" onClick={async () => { await api.approvePackage(p.id); load(); }}>批准</button>{" "}
                  <button className="btn ghost sm" onClick={() => goView("lineage")}>看流转</button>
                </td>
              </tr>
            ))}
            {pending.length === 0 && <tr><td colSpan={4} className="sub">队列空 🎉</td></tr>}
          </table>
        </div>
        <div className="card">
          <h3>⚙️ 工厂配置 <span className="sub">(工序4 媒体当前为 LocalStub 占位)</span></h3>
          <div className="field"><div><label>媒体 Provider</label><div className="hint">工序4 · 配 SINCY_MEDIA_PROVIDERS 接真实生成工具</div></div>
            <select disabled><option>LocalStub (占位)</option></select></div>
          <div className="field"><div><label>平台适配器</label><div className="hint">工序3 · SimpleAdapter 截断+hashtag</div></div>
            <button className="btn ghost sm" onClick={() => goView("devices")}>设备适配器 →</button></div>
          <div className="field mb0"><div><label>工序9 学习回灌</label><div className="hint">factory_learnings → plan_strategy_with_learnings (断点②)</div></div>
            <button className="btn ghost sm" onClick={async () => { try { const r = await api.runFactoryLearn(brands[0]?.id); setMsg(`本次学习结论 ${r.learnings} 条`); } catch (e) { setMsg(String(e)); } }}>跑一次 Learn</button></div>
        </div>
      </div>
    </section>
  );
}
