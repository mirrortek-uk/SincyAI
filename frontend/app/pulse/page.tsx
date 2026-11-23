"use client";

import { useCallback, useEffect, useState } from "react";
import {
  api,
  getToken,
  type PulseEntity,
  type PulseHealth,
  type PulseObservation,
  type PulseProvider,
} from "@/lib/api";

// Sincy Pulse 页: 用户只看到 Sincy 品牌情报产品, 不看到第三方站点。
// Provider 管理 (manifest 注册/启停/手动跑) + 统一实体 (跨源聚合指标+置信度) + 原始观测审计。
export default function PulsePage() {
  const [providers, setProviders] = useState<PulseProvider[]>([]);
  const [health, setHealth] = useState<PulseHealth | null>(null);
  const [entities, setEntities] = useState<PulseEntity[]>([]);
  const [selectedEntity, setSelectedEntity] = useState<string | null>(null);
  const [observations, setObservations] = useState<PulseObservation[]>([]);
  const [typeFilter, setTypeFilter] = useState("");
  const [classFilter, setClassFilter] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [manifestText, setManifestText] = useState(
    JSON.stringify(
      {
        id: "my-provider",
        name: "My Provider",
        platform: "x",
        category: "social_intelligence",
        capabilities: ["account_ranking"],
        acquisition: { type: "api", strategy: "json_api" },
        datasets: [{ id: "main", update_frequency_secs: 3600, entity_type: "social_account" }],
        reliability: 0.7,
        legal_status: "unknown",
      },
      null,
      2,
    ),
  );

  const load = useCallback(async () => {
    if (!getToken()) return;
    try {
      const [p, h, e] = await Promise.all([
        api.listPulseProviders(),
        api.pulseHealth(),
        api.listPulseEntities({
          type: typeFilter || undefined,
          classification: classFilter || undefined,
          limit: 50,
        }),
      ]);
      setProviders(p.items);
      setHealth(h);
      setEntities(e.items);
    } catch (e) {
      setError(String(e));
    }
  }, [typeFilter, classFilter]);

  useEffect(() => {
    load();
  }, [load]);

  async function showObservations(id: string) {
    try {
      const r = await api.listPulseObservations(id);
      setObservations(r.items);
      setSelectedEntity(id);
    } catch (e) {
      setError(String(e));
    }
  }

  async function act(fn: () => Promise<unknown>) {
    setBusy(true);
    setError(null);
    setMsg(null);
    try {
      await fn();
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const legalColor: Record<string, string> = {
    public: "#22c55e",
    commercial_ok: "#22c55e",
    no_redistribute: "#f59e0b",
    attribution_required: "#f59e0b",
    unknown: "#9ca3af",
    blocked: "#ef4444",
  };

  return (
    <main>
      <h1>Sincy Pulse</h1>
      <p className="muted">
        聚合式情报产品: 底层由多个 Intelligence Provider 提供数据, Sincy 统一采集、归一化、
        跨源聚合评分后输出。用户只看到 Sincy Pulse, 原始观测全部保留可审计。本页即世界情报 (/intel) 中的聚合源 <b>sincy-pulse</b> —— 下方 Provider 是可扩展的聚合成员。
        红线: legal_status 非 public/commercial_ok 的 provider 不会被调度。
      </p>
      {error && <p style={{ color: "var(--danger)" }}>{error}</p>}
      {msg && <p style={{ color: "var(--accent)" }}>{msg}</p>}

      {health && (
        <section className="panel">
          <h2>概览</h2>
          <p>
            <b>{health.entities_total}</b> 个统一实体 · <b>{health.observations_total}</b> 条原始观测 ·{" "}
            <b>{health.providers.filter((p) => p.enabled).length}</b>/{health.providers.length} 个 provider 启用
          </p>
        </section>
      )}

      <section className="panel">
        <h2>Intelligence Providers</h2>
        <table>
          <thead>
            <tr><th>Provider</th><th>平台</th><th>capabilities</th><th>法务状态</th><th>健康</th><th>启用</th><th>操作</th></tr>
          </thead>
          <tbody>
            {providers.map((p) => {
              const h = health?.providers.find((x) => x.provider_id === p.id);
              return (
                <tr key={p.id}>
                  <td><b>{p.name}</b><span className="muted"> ({p.id})</span></td>
                  <td>{p.platform}</td>
                  <td className="muted" style={{ fontSize: 11 }}>{p.manifest.capabilities.join(", ")}</td>
                  <td style={{ color: legalColor[p.legal_status] ?? "var(--muted)" }}>{p.legal_status}</td>
                  <td style={{ color: h?.status === "healthy" ? "#22c55e" : h?.status === "failed" ? "#ef4444" : h?.status === "degraded" ? "#f59e0b" : "var(--muted)" }}>
                    {h?.status ?? "-"}
                    {h?.last_error && <span title={h.last_error}> ⚠</span>}
                  </td>
                  <td>{p.enabled ? "✅" : "⏸"}</td>
                  <td>
                    <button disabled={busy} style={{ fontSize: 11, marginRight: 4 }}
                      onClick={() => act(() => api.togglePulseProvider(p.id, !p.enabled))}>
                      {p.enabled ? "停用" : "启用"}
                    </button>
                    <button disabled={busy || !p.enabled} style={{ fontSize: 11 }}
                      onClick={() => act(async () => {
                        const r = await api.runPulseProvider(p.id);
                        setMsg(`${p.id} 采集完成: 抓取 ${r.fetched}, 新实体 ${r.new}${r.errors.length ? `, 失败 ${r.errors.length}` : ""}`);
                      })}>
                      立即采集
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>

        <details style={{ marginTop: 10 }}>
          <summary style={{ cursor: "pointer", fontSize: 13 }}>+ Add Intelligence Provider (上传 manifest)</summary>
          <textarea
            style={{ width: "100%", minHeight: 180, marginTop: 6, fontFamily: "monospace", fontSize: 12 }}
            value={manifestText}
            onChange={(e) => setManifestText(e.target.value)}
          />
          <button disabled={busy} style={{ marginTop: 6 }} onClick={() => act(async () => {
            const m = JSON.parse(manifestText);
            const r = await api.registerPulseProvider(m);
            setMsg(`已注册 ${r.id}, legal_status=${r.legal_status}, enabled=${r.enabled}${r.enabled ? "" : " (未标配合规, 默认不启用)"}`);
          })}>
            注册 Provider
          </button>
        </details>
      </section>

      <section className="panel">
        <h2>统一情报实体 (跨源聚合)</h2>
        <div style={{ display: "flex", gap: 8, marginBottom: 8 }}>
          <select value={typeFilter} onChange={(e) => setTypeFilter(e.target.value)}>
            <option value="">全部类型</option>
            <option value="trend">trend</option>
            <option value="social_post">social_post</option>
            <option value="news">news</option>
            <option value="social_account">social_account</option>
          </select>
          <select value={classFilter} onChange={(e) => setClassFilter(e.target.value)}>
            <option value="">全部趋势</option>
            <option value="viral">🔥 viral</option>
            <option value="rising">📈 rising</option>
            <option value="emerging">🌱 emerging</option>
            <option value="stable">stable</option>
          </select>
          <button onClick={() => load()}>刷新</button>
        </div>
        {entities.length === 0 && <p className="muted">还没有实体 — 对已启用的 provider 点「立即采集」。</p>}
        <table>
          <thead>
            <tr><th>实体</th><th>类型</th><th>跨源分</th><th>置信度</th><th>趋势</th><th>更新时间</th><th>观测</th></tr>
          </thead>
          <tbody>
            {entities.map((e) => {
              const m = e.metrics as Record<string, { estimated?: number; range?: { min: number; max: number }; count?: number }>;
              const firstMetric = Object.values(m)[0];
              const cross = (e.metrics as Record<string, unknown>).cross_source_score as number | undefined;
              const providers = (e.metrics as Record<string, unknown>).provider_count as number | undefined;
              return (
                <tr key={e.id}>
                  <td style={{ maxWidth: 320 }}>
                    <div style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={e.title}>{e.title}</div>
                    <span className="muted" style={{ fontSize: 10 }}>{e.canonical_key.slice(0, 40)}</span>
                  </td>
                  <td>{e.entity_type}</td>
                  <td>{cross !== undefined ? `${cross.toFixed(0)} (${providers} 源)` : "-"}</td>
                  <td>{e.confidence.level ?? "-"} {e.confidence.score !== undefined ? `(${(e.confidence.score * 100).toFixed(0)}%)` : ""}</td>
                  <td>{e.trend.classification ?? "-"}{firstMetric?.estimated !== undefined ? ` · ${Math.round(firstMetric.estimated).toLocaleString()}` : ""}</td>
                  <td className="muted">{e.updated_at.slice(0, 19).replace("T", " ")}</td>
                  <td><button style={{ fontSize: 11 }} onClick={() => showObservations(e.id)}>查看</button></td>
                </tr>
              );
            })}
          </tbody>
        </table>

        {selectedEntity && (
          <div style={{ marginTop: 10 }}>
            <h3>原始观测 (可审计)</h3>
            <table>
              <thead><tr><th>provider</th><th>metric</th><th>value</th><th>时间</th></tr></thead>
              <tbody>
                {observations.map((o, i) => (
                  <tr key={i}>
                    <td>{o.provider_id}</td>
                    <td>{o.metric}</td>
                    <td>{JSON.stringify(o.value)}</td>
                    <td className="muted">{o.observed_at.slice(0, 19).replace("T", " ")}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </main>
  );
}
