"use client";

import { useCallback, useEffect, useState } from "react";
import {
  api,
  getToken,
  type CollectResult,
  type IntelItem,
  type IntelJob,
  type IntelProvider,
  type IntelStats,
  type IntelSource,
  type OpportunityItem,
} from "@/lib/api";

// 世界情报页 (WI guide): 信源注册表 / Provider 路由与健康 / 采集条目 /
// 机会引擎。只读公共信源, 不绕过任何平台的认证、验证码与限流。
export default function IntelPage() {
  const [sources, setSources] = useState<IntelSource[]>([]);
  const [providers, setProviders] = useState<IntelProvider[]>([]);
  const [items, setItems] = useState<IntelItem[]>([]);
  const [opps, setOpps] = useState<OpportunityItem[]>([]);
  const [jobs, setJobs] = useState<IntelJob[]>([]);
  const [stats, setStats] = useState<IntelStats | null>(null);
  const [collectResult, setCollectResult] = useState<CollectResult | null>(null);
  const [collecting, setCollecting] = useState(false);
  const [capability, setCapability] = useState("social_search");
  const [platform, setPlatform] = useState("x");
  const [route, setRoute] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!getToken()) return;
    try {
      const [s, p, i, o, j, st] = await Promise.all([
        api.listIntelSources(),
        api.listIntelProviders(),
        api.listIntelItems(undefined, 50),
        api.listOpportunities(30),
        api.listIntelJobs(),
        api.intelStats(),
      ]);
      setSources(s.items);
      setProviders(p.items);
      setItems(i.items);
      setOpps(o.items);
      setJobs(j.items);
      setStats(st);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    load();
    const t = setInterval(load, 30_000); // 采集是后台持续跑的, 页面 30s 自动刷新
    return () => clearInterval(t);
  }, [load]);

  return (
    <main>
      <h1>世界情报</h1>
      <p className="muted">
        全局信源与 Provider 路由 (WI §6/§8), 机会引擎输出 (WI §19) 供数字员工消费 (WI §20)。
      </p>
      {error && <p style={{ color: "var(--danger)" }}>{error}</p>}
      {msg && <p className="muted">{msg}</p>}

      <section className="panel">
        <h2>Provider 路由测试</h2>
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          <select value={capability} onChange={(e) => setCapability(e.target.value)}>
            {["social_search", "trend_search", "web_search", "news_research", "web_extraction", "creator_research"].map((c) => (
              <option key={c}>{c}</option>
            ))}
          </select>
          <input value={platform} onChange={(e) => setPlatform(e.target.value)} style={{ width: 80 }} />
          <button
            onClick={async () => {
              try {
                const r = await api.intelRoute({ capability, platform });
                setRoute(`${r.providers.map((p) => p.name).join(" → ")} (${r.reason})`);
              } catch (e) {
                setRoute(`无可用 provider: ${String(e)}`);
              }
            }}
          >
            路由
          </button>
          <button
            disabled={collecting}
            onClick={async () => {
              setCollecting(true);
              try {
                const r = await api.collectNow();
                setCollectResult(r);
                await load();
              } catch (e) {
                setError(String(e));
              } finally {
                setCollecting(false);
              }
            }}
          >
            {collecting ? "采集中…" : "立即采集一轮"}
          </button>
          <button
            onClick={async () => {
              try {
                const r = await api.refreshOpportunities();
                setMsg(`机会引擎: ${r.opportunities} 个候选, 写入 ${r.upserted} 条`);
                await load();
              } catch (e) {
                setError(String(e));
              }
            }}
          >
            刷新机会
          </button>
        </div>
        {route && <p className="muted">路由结果: {route}</p>}
        {collectResult && (
          <p className="muted">
            本轮采集: 新入库 {collectResult.inserted} 条 · 去重 {collectResult.duplicates} 条 · 失败 {collectResult.failures.length} 条
            {collectResult.failures.length > 0 && <span style={{ color: "#ef4444" }}>({collectResult.failures[0].slice(0, 60)})</span>}
            {" "}· 机会引擎 {collectResult.opportunities} 个候选, 写入 {collectResult.opportunities_upserted} 条
          </p>
        )}
      </section>

      <section className="panel">
        <h2>机会 (opportunities)</h2>
        {opps.length === 0 && <p className="muted">暂无机会, 先点"刷新机会"。</p>}
        <table>
          <thead><tr><th>主题</th><th>得分</th><th>状态</th><th>证据</th></tr></thead>
          <tbody>
            {opps.map((o) => (
              <tr key={o.id}>
                <td>{o.topic}</td>
                <td>{Number(o.score).toFixed(3)}</td>
                <td>{o.status}</td>
                <td className="muted">{o.evidence.join(", ").slice(0, 60)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section className="panel">
        <h2>采集任务 (后台定时跑, 可手动触发)</h2>
        {jobs.length === 0 && <p className="muted">暂无任务记录。</p>}
        <table>
          <thead><tr><th>信源</th><th>计划</th><th>状态</th><th>最近运行</th><th>最近错误</th></tr></thead>
          <tbody>
            {jobs.map((j, i) => (
              <tr key={i}>
                <td>{j.source_id}</td>
                <td className="muted">{j.schedule}</td>
                <td style={{ color: j.status === "failed" ? "#ef4444" : undefined }}>{j.status}</td>
                <td className="muted">{j.last_run_at ? j.last_run_at.slice(0, 19).replace("T", " ") : "-"}{j.last_inserted != null ? ` (+${j.last_inserted})` : ""}</td>
                <td className="muted" style={{ color: "#ef4444" }}>{j.last_error ? j.last_error.slice(0, 50) : ""}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section className="panel">
        <h2>信源注册表</h2>
        <table>
          <thead><tr><th>id</th><th>名称</th><th>类别</th><th>能力</th><th>平台</th><th>启用</th></tr></thead>
          <tbody>
            {sources.map((s) => (
              <tr key={s.id}>
                <td>{s.id}</td>
                <td>{s.name}</td>
                <td>{s.category}</td>
                <td className="muted">{s.capabilities.join(",")}</td>
                <td className="muted">{s.platforms.join(",") || "-"}</td>
                <td>{s.enabled ? "是" : "否"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section className="panel">
        <h2>Providers</h2>
        <table>
          <thead><tr><th>名称</th><th>信源</th><th>健康</th><th>成本</th><th>限流</th><th>操作</th></tr></thead>
          <tbody>
            {providers.map((p) => (
              <tr key={p.id}>
                <td>{p.name}</td>
                <td>{p.source_id}</td>
                <td>{p.health}</td>
                <td className="muted">{JSON.stringify(p.cost_model)}</td>
                <td className="muted">{JSON.stringify(p.rate_limit)}</td>
                <td>
                  <button
                    onClick={async () => {
                      try {
                        const r = await api.providerHealth(p.id);
                        setMsg(`${r.name}: ${r.health} (${r.latency_ms}ms) ${r.detail}`);
                        await load();
                      } catch (e) {
                        setError(String(e));
                      }
                    }}
                  >
                    健康检查
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section className="panel">
        <h2>最近采集条目</h2>
        <table>
          <thead><tr><th>kind</th><th>信源</th><th>内容</th><th>时间</th></tr></thead>
          <tbody>
            {items.map((i) => {
              const p = i.payload as Record<string, unknown>;
              const title = (p.title as string) ?? (p.topic as string) ?? "";
              const extra = p.volume !== undefined ? ` ★${p.volume} vel${p.velocity}` : (p.summary as string) ?? (p.content as string) ?? "";
              return (
                <tr key={i.id}>
                  <td>{i.kind}</td>
                  <td>{i.source_id}</td>
                  <td>
                    {p.url ? (
                      <a href={p.url as string} target="_blank" rel="noreferrer" style={{ color: "var(--accent)" }}>{title.slice(0, 60)}</a>
                    ) : (
                      title.slice(0, 60)
                    )}
                    <span className="muted"> {String(extra).slice(0, 80)}</span>
                  </td>
                  <td className="muted">{i.collected_at.slice(0, 19).replace("T", " ")}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </section>
    </main>
  );
}
