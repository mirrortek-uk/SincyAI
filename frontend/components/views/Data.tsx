"use client";

// 数据流视图: 世界情报 / 信号机会 / 信源健康 A/B/C / 待你处理

import { useEffect, useState } from "react";

import {
  api,
  type CustomFeed,
  type IntelSignal,
  type IntelSourceStatusResp,
  type IntelStats,
  type OpportunityItem,
  type PackageItem,
  type PulseProvider,
  type TaskItem,
} from "@/lib/api";
import { goView } from "@/components/Shell";
import { Badge, Err, Loading, healthBadge } from "@/components/ui";

// ---------- 🌐 世界情报 ----------
export function IntelView() {
  const [stats, setStats] = useState<IntelStats | null>(null);
  const [providers, setProviders] = useState<PulseProvider[] | null>(null);
  const [signals, setSignals] = useState<IntelSignal[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    Promise.all([api.intelStats(), api.listPulseProviders(), api.listIntelSignals(undefined, 20)])
      .then(([s, p, g]) => { setStats(s); setProviders(p.items); setSignals(g.items); })
      .catch((e) => setErr(String(e)));
  }, []);
  if (err) return <Err e={err} />;
  if (!stats || !providers || !signals) return <Loading />;
  return (
    <section className="view active">
      <h1>🌐 世界情报</h1>
      <p className="lede">内容工厂的原料来源。统一采集 → 归一 → 跨源聚合 → Signals，数字员工只消费 Signal/机会。</p>
      <div className="grid g4">
        <div className="card"><div className="sub">情报记录</div><div className="kpi">{stats.total_items.toLocaleString()}</div></div>
        <div className="card"><div className="sub">按类型</div><div className="kpi" style={{ fontSize: 18 }}>{stats.by_kind.map((k) => `${k.kind}:${k.n}`).join(" · ")}</div></div>
        <div className="card"><div className="sub">启用 provider</div><div className="kpi">{providers.filter((p) => p.enabled).length}/{providers.length}</div></div>
        <div className="card"><div className="sub">最近信号</div><div className="kpi">{signals.length}</div></div>
      </div>
      <div className="card mt14"><h3>Intelligence Providers (Sincy Pulse)</h3>
        <table><tr><th>Provider</th><th>平台</th><th>capabilities</th><th>健康</th><th>启用</th></tr>
          {providers.map((p) => (
            <tr key={p.id}>
              <td>{p.name}</td>
              <td>{p.platform}</td>
              <td className="sub">{(p.manifest?.capabilities ?? []).slice(0, 4).join(", ")}</td>
              <td>{p.legal_status === "public" || p.legal_status === "commercial_ok" ? <Badge kind="ok">{p.legal_status}</Badge> : <Badge kind="warn">{p.legal_status}</Badge>}</td>
              <td>{p.enabled ? <Badge kind="ok">on</Badge> : <Badge kind="pend">off</Badge>}</td>
            </tr>
          ))}
        </table>
        <div className="sub mt14">红线: legal_status 非 public/commercial_ok 的 provider 不会被调度。</div>
      </div>
      <div className="card mt14"><h3>最近信号</h3>
        <table><tr><th>类型</th><th>强度</th><th>来源</th><th>时间</th><th>摘要</th></tr>
          {signals.slice(0, 12).map((s) => (
            <tr key={s.signal_id}>
              <td><Badge kind="pend">{s.signal_type}</Badge></td>
              <td>{s.strength}</td>
              <td className="sub">{s.source_id}</td>
              <td className="sub">{s.observed_at.slice(5, 16).replace("T", " ")}</td>
              <td className="sub">{String((s.payload as Record<string, unknown>)?.title ?? "").slice(0, 50)}</td>
            </tr>
          ))}
        </table>
      </div>
    </section>
  );
}

// ---------- 📡 信号 / 机会 ----------
export function SignalsView() {
  const [opps, setOpps] = useState<OpportunityItem[] | null>(null);
  const [signals, setSignals] = useState<IntelSignal[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [seg, setSeg] = useState("全部");
  useEffect(() => {
    Promise.all([api.listOpportunities(50), api.listIntelSignals(undefined, 30)])
      .then(([o, s]) => { setOpps(o.items); setSignals(s.items); })
      .catch((e) => setErr(String(e)));
  }, []);
  if (err) return <Err e={err} />;
  if (!opps || !signals) return <Loading />;
  const filtered = opps.filter((o) => seg === "全部" || o.status === seg);
  const mkBrief = async (o: OpportunityItem) => {
    setMsg(null);
    try {
      const brands = await api.listBrands();
      await api.createFactoryBrief({
        trigger: "opportunity_driven",
        brand_id: brands.items[0]?.id ?? "",
        opportunity_id: o.id,
        goal: o.topic,
        platforms: ["x"],
      });
      setMsg(`已为「${o.topic.slice(0, 20)}」建 Brief ✅`);
    } catch (e) { setMsg(String(e)); }
  };
  return (
    <section className="view active">
      <h1>📡 信号 / 机会</h1>
      <p className="lede">entity → signal → opportunity。机会引擎按 pillar_match + 热度打分，数字员工据此选题。</p>
      <div className="card">
        <h3>机会 (opportunity) <span className="sp" />
          <div className="seg">
            {["全部", "new", "selected", "published"].map((s) => (
              <button key={s} className={seg === s ? "on" : ""} onClick={() => setSeg(s)}>{s}</button>
            ))}
          </div></h3>
        {msg && <p className="sub mb0">{msg}</p>}
        <table><tr><th>话题</th><th>分数</th><th>pillar_match</th><th>状态</th><th></th></tr>
          {filtered.slice(0, 20).map((o) => {
            const r = o.rationale as Record<string, unknown>;
            return (
              <tr key={o.id}>
                <td>{o.topic.slice(0, 48)}</td>
                <td><b>{o.score}</b></td>
                <td className="sub">{String(r.pillar_match ?? "—")}</td>
                <td>{o.status === "published" ? <Badge kind="ok">published</Badge>
                  : o.status === "selected" ? <Badge kind="run">selected</Badge>
                  : <Badge kind="pend">{o.status}</Badge>}</td>
                <td>
                  {o.status !== "published" && <button className="btn ghost sm" onClick={() => mkBrief(o)}>建 Brief</button>}{" "}
                  <button className="btn ghost sm" onClick={() => goView("lineage")}>看流转</button>
                </td>
              </tr>
            );
          })}
          {filtered.length === 0 && <tr><td colSpan={5} className="sub">空</td></tr>}
        </table>
      </div>
      <div className="card mt14"><h3>最新信号</h3>
        <table><tr><th>类型</th><th>强度</th><th>来源</th><th>时间</th></tr>
          {signals.slice(0, 15).map((s) => (
            <tr key={s.signal_id}><td>{s.signal_type}</td><td>{s.strength}</td><td className="sub">{s.source_id}</td><td className="sub">{s.observed_at.slice(5, 16).replace("T", " ")}</td></tr>
          ))}
        </table>
      </div>
    </section>
  );
}

// ---------- 🩺 信源健康 A/B/C ----------
export function SourcesView() {
  const [data, setData] = useState<IntelSourceStatusResp | null>(null);
  const [feeds, setFeeds] = useState<CustomFeed[] | null>(null);
  const [importUrl, setImportUrl] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const load = () => {
    api.intelSourceStatus().then(setData).catch((e) => setErr(String(e)));
    api.listCustomFeeds().then((d) => setFeeds(d.feeds)).catch(() => {});
  };
  useEffect(load, []);
  if (err) return <Err e={err} />;
  if (!data) return <Loading />;
  const A = data.sources.filter((s) => s.class === "A");
  const B = data.sources.filter((s) => s.class === "B");
  const C = data.sources.filter((s) => s.class === "C");
  const healthy = data.sources.filter((s) => s.health === "healthy").length;
  const degraded = data.sources.filter((s) => s.health === "degraded").length;
  const failed = data.sources.filter((s) => s.health === "failed").length;
  const doImport = async () => {
    setMsg(null);
    try {
      const r = await api.importFeedList(importUrl.trim(), true);
      setMsg(`导入完成: 发现 ${r.found} 条, 新增 ${r.added}, 跳过 ${r.skipped_existing}`);
      setImportUrl(""); load();
    } catch (e) { setMsg(String(e)); }
  };
  return (
    <section className="view active">
      <h1>🩺 信源健康 A/B/C</h1>
      <p className="lede">A = 跑在 HttpConnector 上 · B = aggregator 走 pulse · C = 声明但缺配置。加源 = 多一个 SourceRef (RSS 零代码 / 列表批量导入)。</p>
      <div className="grid g4">
        <div className="card"><div className="sub">A 跑通</div><div className="kpi" style={{ color: "var(--ok)" }}>{healthy}</div></div>
        <div className="card"><div className="sub">degraded</div><div className="kpi" style={{ color: "var(--warn)" }}>{degraded}</div></div>
        <div className="card"><div className="sub">failed</div><div className="kpi" style={{ color: "var(--bad)" }}>{failed}</div></div>
        <div className="card"><div className="sub">C 未配置</div><div className="kpi" style={{ color: "var(--pend)" }}>{C.length}</div></div>
      </div>
      <div className="card mt14">
        <h3>信源 <span className="sub">共 {data.sources.length} (A {A.length} · B {B.length} · C {C.length})</span> <span className="sp" />
          <input type="text" placeholder="粘贴 feed 列表 URL (pressat/OPML…)" value={importUrl} onChange={(e) => setImportUrl(e.target.value)} style={{ width: 260 }} />
          <button className="btn ghost sm" onClick={doImport} disabled={!importUrl.trim()}>导入 feed 列表</button>
        </h3>
        {msg && <p className="sub">{msg}</p>}
        <table><tr><th>源</th><th>类</th><th>connector</th><th>健康</th><th>累计入库</th><th>缺什么</th></tr>
          {data.sources.map((s) => (
            <tr key={s.source_id}>
              <td>{s.name}</td>
              <td><Badge kind={s.class === "A" ? "ok" : s.class === "B" ? "run" : "pend"}>{s.class}</Badge></td>
              <td className="sub">{s.connector ?? "—"}{s.connector_type ? ` (${s.connector_type})` : ""}</td>
              <td>{healthBadge(s.health)}</td>
              <td>{s.items_total.toLocaleString()}</td>
              <td className="sub">{s.missing ?? (s.health === "healthy" ? "—" : s.health)}</td>
            </tr>
          ))}
        </table>
      </div>
      {feeds && feeds.length > 0 && (
        <div className="card mt14"><h3>自定义 RSS feed ({feeds.length})</h3>
          <table><tr><th>标签</th><th>URL</th><th>来源</th><th>启用</th><th>入库</th></tr>
            {feeds.map((f) => (
              <tr key={f.id}>
                <td>{f.label}</td><td className="sub" style={{ maxWidth: 320, overflow: "hidden", textOverflow: "ellipsis" }}>{f.url}</td>
                <td className="sub">{f.origin ?? "—"}</td>
                <td>{f.enabled ? <Badge kind="ok">on</Badge> : <Badge kind="pend">off</Badge>}</td>
                <td>{f.items_total}</td>
              </tr>
            ))}
          </table>
        </div>
      )}
    </section>
  );
}

// ---------- ✅ 待你处理 ----------
export function ApprovalsView() {
  const [pkgs, setPkgs] = useState<PackageItem[] | null>(null);
  const [tasks, setTasks] = useState<TaskItem[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const load = () => {
    Promise.all([api.listPackages(), api.listTasks()])
      .then(([p, t]) => { setPkgs(p.items); setTasks(t.items); })
      .catch((e) => setErr(String(e)));
  };
  useEffect(load, []);
  if (err) return <Err e={err} />;
  if (!pkgs || !tasks) return <Loading />;
  const pending = pkgs.filter((p) => p.approval_status === "pending");
  const waiting = tasks.filter((t) => t.state === "WAITING_USER");
  return (
    <section className="view active">
      <h1>✅ 待你处理</h1>
      <p className="lede">所有需要人工介入的点集中在这里：内容审批、设备人机验证、配置缺口。</p>
      {msg && <p className="sub">{msg}</p>}
      <div className="card">
        <h3>内容审批 <span className="tag t-fac">工序6</span> <span className="sub">{pending.length} 条</span></h3>
        <table><tr><th>内容</th><th>平台</th><th>来源</th><th>创建</th><th>操作</th></tr>
          {pending.map((p) => (
            <tr key={p.id}>
              <td>{p.text_preview.slice(0, 48)}</td>
              <td>{p.platform}</td>
              <td><Badge kind="pend">{p.constraints?.source ?? "—"}</Badge></td>
              <td className="sub">{p.created_at.slice(5, 16).replace("T", " ")}</td>
              <td>
                <button className="btn sm" onClick={async () => { try { await api.approvePackage(p.id); setMsg("已批准 ✅"); load(); } catch (e) { setMsg(String(e)); } }}>批准发布</button>{" "}
                <button className="btn ghost sm" onClick={() => goView("lineage")}>看流转</button>
              </td>
            </tr>
          ))}
          {pending.length === 0 && <tr><td colSpan={5} className="sub">没有待审批内容 🎉</td></tr>}
        </table>
      </div>
      <div className="card mt14">
        <h3>设备人机验证 <span className="sub">{waiting.length} 个</span></h3>
        {waiting.length === 0 && <p className="sub">没有 WAITING_USER 任务 ✅</p>}
        {waiting.map((t) => (
          <div className="alert b" key={t.id}>🛑 设备任务「{t.action ?? t.id.slice(0, 8)}」出现人机验证 → 已自动转人工 (从不自动过, 守 ToS)。解锁手机后点重试。
            <span className="sp" />
            <button className="btn ghost sm" onClick={async () => { try { await api.retryTask(t.id); setMsg("已重试"); load(); } catch (e) { setMsg(String(e)); } }}>已人工处理, 重试</button>
          </div>
        ))}
      </div>
    </section>
  );
}
