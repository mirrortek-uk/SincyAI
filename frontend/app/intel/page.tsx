"use client";

import { useCallback, useEffect, useState } from "react";
import {
  api,
  getToken,
  type IntelItem,
  type IntelProvider,
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
  const [capability, setCapability] = useState("social_search");
  const [platform, setPlatform] = useState("x");
  const [route, setRoute] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!getToken()) return;
    try {
      const [s, p, i, o] = await Promise.all([
        api.listIntelSources(),
        api.listIntelProviders(),
        api.listIntelItems(undefined, 30),
        api.listOpportunities(30),
      ]);
      setSources(s.items);
      setProviders(p.items);
      setItems(i.items);
      setOpps(o.items);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  return (
    <main>
      <h1>世界情报</h1>
      <p className="muted">
        全局信源与 Provider 路由 (WI §6/§8), 机会引擎输出 (WI §19) 供数字员工消费 (WI §20)。
      </p>
      {error && <p style={{ color: "var(--danger)" }}>{error}</p>}
      {msg && <p className="muted">{msg}</p>}

      <section className="card">
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
      </section>

      <section className="card">
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

      <section className="card">
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

      <section className="card">
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

      <section className="card">
        <h2>最近采集条目</h2>
        <table>
          <thead><tr><th>kind</th><th>信源</th><th>dedup_key</th><th>时间</th></tr></thead>
          <tbody>
            {items.map((i) => (
              <tr key={i.id}>
                <td>{i.kind}</td>
                <td>{i.source_id}</td>
                <td className="muted">{i.dedup_key}</td>
                <td className="muted">{i.collected_at.slice(0, 19).replace("T", " ")}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
    </main>
  );
}
