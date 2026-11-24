"use client";

import { useCallback, useEffect, useState } from "react";
import {
  api,
  getToken,
  type CollectResult,
  type CustomerIntelProfile,
  type IndustryDef,
  type IntelSignal,
  type IntelPipeline,
  type BrandItem,
  type IntelHealth,
  type IntelSourceStatusResp,
  type CustomFeed,
  type FeedImportResult,
  type IntelItem,
  type IntelJob,
  type IntelProvider,
  type IntelStats,
  type IntelSource,
  type IntelTrend,
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
  const [health, setHealth] = useState<IntelHealth | null>(null);
  const [srcStatus, setSrcStatus] = useState<IntelSourceStatusResp | null>(null);
  const [customFeeds, setCustomFeeds] = useState<CustomFeed[]>([]);
  const [importUrl, setImportUrl] = useState("");
  const [importResult, setImportResult] = useState<FeedImportResult | null>(null);
  const [importing, setImporting] = useState(false);
  const [trends, setTrends] = useState<IntelTrend[]>([]);
  const [collectResult, setCollectResult] = useState<CollectResult | null>(null);
  const [collecting, setCollecting] = useState(false);
  const [capability, setCapability] = useState("social_search");
  const [platform, setPlatform] = useState("x");
  const [route, setRoute] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Sincy-04: 行业过滤 + 客户画像 + 相关性 feed
  const [industries, setIndustries] = useState<IndustryDef[]>([]);
  const [industryFilter, setIndustryFilter] = useState("");
  const [brands, setBrands] = useState<BrandItem[]>([]);
  const [brandId, setBrandId] = useState("");
  const [profile, setProfile] = useState<CustomerIntelProfile>({});
  const [relevanceMode, setRelevanceMode] = useState(false);
  const [signals, setSignals] = useState<IntelSignal[]>([]);
  const [pipelines, setPipelines] = useState<IntelPipeline[]>([]);

  const load = useCallback(async () => {
    if (!getToken()) return;
    try {
      const [s, p, i, o, j, st, h, tr, ind, br, sig, pipe, ss, cf] = await Promise.all([
        api.listIntelSources(),
        api.listIntelProviders(),
        relevanceMode && brandId
          ? api.brandIntelFeed(brandId, 50).then((f) => ({ items: f.items }))
          : api.listIntelItems(undefined, 50, industryFilter || undefined),
        api.listOpportunities(30),
        api.listIntelJobs(),
        api.intelStats(),
        api.intelHealth(),
        api.intelTrends(),
        api.listIndustries(),
        api.listBrands(),
        api.listIntelSignals(undefined, 30),
        api.listIntelPipelines(),
        api.intelSourceStatus(),
        api.listCustomFeeds(),
      ]);
      setSources(s.items);
      setProviders(p.items);
      setItems(i.items);
      setOpps(o.items);
      setJobs(j.items);
      setStats(st);
      setHealth(h);
      setTrends(tr.trends);
      setIndustries(ind.items);
      setBrands(br.items);
      setSignals(sig.items);
      setPipelines(pipe.pipelines);
      setSrcStatus(ss);
      setCustomFeeds(cf.feeds);
    } catch (e) {
      setError(String(e));
    }
  }, [industryFilter, relevanceMode, brandId]);

  useEffect(() => {
    load();
    const t = setInterval(load, 30_000); // 采集是后台持续跑的, 页面 30s 自动刷新
    return () => clearInterval(t);
  }, [load]);

  useEffect(() => {
    if (!brandId) return;
    api.getIntelProfile(brandId).then((r) => setProfile(r.profile)).catch(() => setProfile({}));
  }, [brandId]);

  const saveProfile = async () => {
    if (!brandId) return;
    try {
      await api.putIntelProfile(brandId, profile);
      setMsg("画像已保存");
      load();
    } catch (e) {
      setError(String(e));
    }
  };

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
        <h2>信源接入全景 (A/B/C 分类)</h2>
        <p className="muted" style={{ marginTop: 0 }}>
          A 类 = 绑在真跑着的 HttpConnector 上 · B 类 = aggregator 走 Sincy Pulse · C 类 = 已注册但未接通抓取路径 (not_configured)。
          {srcStatus && <> 当前 <b>A:{srcStatus.summary.A}</b> · <b>B:{srcStatus.summary.B}</b> · <b>C:{srcStatus.summary.C}</b></>}
        </p>
        {srcStatus && (["A", "B", "C"] as const).map((cls) => {
          const rows = srcStatus.sources.filter((x) => x.class === cls);
          if (rows.length === 0) return null;
          const titles: Record<string, string> = {
            A: "A 类 · 绑在真跑着的 HttpConnector 上",
            B: "B 类 · aggregator, 走 pulse (不走 HttpConnector)",
            C: "C 类 · 声明了但没接通抓取路径 → not_configured",
          };
          return (
            <div key={cls} style={{ marginBottom: 12 }}>
              <h3 style={{ marginBottom: 4 }}>{titles[cls]} ({rows.length})</h3>
              <table>
                <thead><tr><th>源</th><th>健康</th><th>累计入库</th><th>挂在哪个 connector</th><th>类型</th><th>解析路径</th><th>缺什么</th></tr></thead>
                <tbody>
                  {rows.map((r) => (
                    <tr key={r.source_id}>
                      <td>{r.source_id}<span className="muted"> · {r.name}</span></td>
                      <td style={{ color: r.health === "healthy" ? "#22c55e" : r.health === "failed" ? "#ef4444" : r.health === "degraded" ? "#f59e0b" : "var(--muted)" }}>{r.health}</td>
                      <td>{r.items_total}</td>
                      <td className="muted">{r.connector ?? "-"}</td>
                      <td className="muted">{r.connector_type ?? r.category}</td>
                      <td className="muted">{r.parse_path ?? "-"}</td>
                      <td className="muted" style={{ color: "#f59e0b" }}>{r.missing ?? "—"}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          );
        })}
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap", alignItems: "center", marginTop: 8 }}>
          <input
            placeholder="导入清单页 URL (如 awesome-rss-feeds 的 LIST.md / checkrss / rss-guru)"
            style={{ minWidth: 420 }}
            value={importUrl}
            onChange={(e) => setImportUrl(e.target.value)}
          />
          <button
            disabled={importing || !importUrl}
            onClick={async () => {
              setImporting(true);
              try {
                const r = await api.importFeedList(importUrl, false);
                setImportResult(r);
                setMsg(`预览: 从清单页抽取到 ${r.found} 个 feed URL (未写入, 点"确认导入"生效)`);
              } catch (e) { setError(String(e)); } finally { setImporting(false); }
            }}
          >
            预览清单
          </button>
          <button
            disabled={importing || !importUrl}
            onClick={async () => {
              setImporting(true);
              try {
                const r = await api.importFeedList(importUrl, true);
                setImportResult(r);
                setMsg(`已导入: 新增 ${r.added} 个自定义源, 跳过已存在 ${r.skipped_existing} 个 (rss-custom 连接器下一轮自动抓取)`);
                await load();
              } catch (e) { setError(String(e)); } finally { setImporting(false); }
            }}
          >
            确认导入
          </button>
        </div>
        {importResult && (
          <p className="muted">清单 {importResult.list_url}: 抽取 {importResult.found} 个 feed{importResult.apply ? `, 新增 ${importResult.added}, 已存在 ${importResult.skipped_existing}` : ""}。</p>
        )}
        {customFeeds.length > 0 && (
          <details style={{ marginTop: 8 }}>
            <summary className="muted">自定义 RSS 源 ({customFeeds.length} 个, intel_custom_feeds 表, rss-custom 连接器抓取)</summary>
            <table>
              <thead><tr><th>id</th><th>label</th><th>url</th><th>来源</th><th>启用</th><th>累计入库</th></tr></thead>
              <tbody>
                {customFeeds.map((f) => (
                  <tr key={f.id}>
                    <td>{f.id}</td>
                    <td>{f.label}</td>
                    <td className="muted"><a href={f.url} target="_blank" rel="noreferrer" style={{ color: "var(--accent)" }}>{f.url.slice(0, 70)}</a></td>
                    <td className="muted">{f.origin ?? "手工"}</td>
                    <td>{f.enabled ? "是" : "否"}</td>
                    <td>{f.items_total}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </details>
        )}
      </section>

      <section className="panel">
        <h2>
          信源健康审计 (Source Health Audit)
          {" "}
          <button
            style={{ fontSize: 12 }}
            onClick={async () => {
              try {
                const r = await api.enableAllIntelSources();
                const j = await api.resumeAllIntelJobs();
                setMsg(`已启用全部信源 (${r.enabled} 个), 恢复 paused 任务 ${j.resumed} 个`);
                await load();
              } catch (e) { setError(String(e)); }
            }}
          >
            一键启用全部信源
          </button>
        </h2>
        <p className="muted" style={{ marginTop: 0 }}>
          状态含义: healthy=正常入库 · degraded=有错误但仍在跑 · failed=最近全失败 ·
          not_configured=已启用但缺外部配置(自托管 RSSHub/SearXNG 或 provider key) · disabled=已停用。
        </p>
        {health && (
          <>
            <p>
              {Object.entries(health.summary).map(([k, v]) => (
                <span key={k} style={{ marginRight: 12 }}>
                  <b style={{ color: k === "healthy" ? "#22c55e" : k === "failed" ? "#ef4444" : k === "degraded" ? "#f59e0b" : "var(--muted)" }}>
                    {k}: {v}
                  </b>
                </span>
              ))}
            </p>
            <table>
              <thead><tr><th>信源</th><th>健康状态</th><th>jobs</th><th>已运行</th><th>累计入库</th><th>上轮新增</th><th>最近错误</th><th>操作</th></tr></thead>
              <tbody>
                {health.sources.map((s) => (
                  <tr key={s.source_id}>
                    <td>{s.source_id}<span className="muted"> · {s.name}</span></td>
                    <td style={{ color: s.health === "healthy" ? "#22c55e" : s.health === "failed" ? "#ef4444" : s.health === "degraded" ? "#f59e0b" : "var(--muted)" }}>{s.health}</td>
                    <td>{s.jobs}</td>
                    <td>{s.jobs_ran}</td>
                    <td>{s.items_total}</td>
                    <td>{s.last_new ?? "-"}</td>
                    <td className="muted" style={{ color: "#ef4444" }}>{s.last_error ? s.last_error.slice(0, 60) : ""}</td>
                    <td>
                      <button
                        style={{ fontSize: 11 }}
                        onClick={async () => {
                          try {
                            await api.toggleIntelSource(s.source_id, !s.enabled);
                            await load();
                          } catch (e) { setError(String(e)); }
                        }}
                      >
                        {s.enabled ? "停用" : "启用"}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </>
        )}
      </section>

      <section className="panel">
        <h2>趋势情报 (Trend Intelligence · 最近 24h)</h2>
        {trends.length === 0 && <p className="muted">暂无足够数据 (同一主题需 ≥3 条)。</p>}
        <table>
          <thead><tr><th>主题</th><th>24h 总量</th><th>近 6h</th><th>速度/h</th><th>增长率</th><th>跨平台</th><th>来源</th></tr></thead>
          <tbody>
            {trends.slice(0, 15).map((t) => (
              <tr key={t.topic}>
                <td title={t.samples.join("\n")}>{t.topic.slice(0, 60)}</td>
                <td>{t.total_24h}</td>
                <td>{t.last_6h}</td>
                <td>{t.velocity_per_h}</td>
                <td style={{ color: t.growth_pct > 50 ? "#22c55e" : t.growth_pct < 0 ? "#ef4444" : undefined }}>
                  {t.growth_pct > 0 ? "+" : ""}{t.growth_pct}%
                </td>
                <td>{t.platforms}</td>
                <td className="muted">{t.sources.slice(0, 4).join(", ")}</td>
              </tr>
            ))}
          </tbody>
        </table>
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
          <thead><tr><th>信源</th><th>健康</th><th>计划</th><th>状态</th><th>最近运行 (抓到/新增)</th><th>最近错误</th></tr></thead>
          <tbody>
            {jobs.map((j, i) => {
              const h = health?.sources.find((s) => s.source_id === j.source_id);
              return (
              <tr key={i}>
                <td>{j.source_id}</td>
                <td className="muted" style={{ color: h && h.health !== "healthy" && h.health !== "disabled" ? "#f59e0b" : undefined }}>{h?.health ?? "-"}</td>
                <td className="muted">{j.schedule}</td>
                <td style={{ color: j.status === "failed" ? "#ef4444" : undefined }}>{j.status}</td>
                <td className="muted">{j.last_run_at ? j.last_run_at.slice(0, 19).replace("T", " ") : "-"}{j.last_fetched != null ? ` (${j.last_fetched}/${j.last_new ?? 0})` : j.last_inserted != null ? ` (+${j.last_inserted})` : ""}</td>
                <td className="muted" style={{ color: "#ef4444" }}>{j.last_error ? j.last_error.slice(0, 50) : ""}</td>
              </tr>
              );
            })}
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
                <td>{s.category === "aggregate" ? <span title="聚合情报源: 由 Sincy Pulse 跨源聚合输出">聚合源</span> : s.category}</td>
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
        <h2>客户情报画像 (Sincy-04)</h2>
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap", alignItems: "center" }}>
          <select value={brandId} onChange={(e) => setBrandId(e.target.value)}>
            <option value="">选择品牌…</option>
            {brands.map((b) => (
              <option key={b.id} value={b.id}>{b.name}</option>
            ))}
          </select>
          <select
            value={profile.industry ?? ""}
            onChange={(e) => setProfile({ ...profile, industry: e.target.value })}
          >
            <option value="">一级行业…</option>
            {industries.filter((x) => x.level === 1).map((x) => (
              <option key={x.id} value={x.id}>{x.name}</option>
            ))}
          </select>
          <select
            value={profile.sub_industry ?? ""}
            onChange={(e) => setProfile({ ...profile, sub_industry: e.target.value })}
          >
            <option value="">子行业…</option>
            {industries.filter((x) => x.level === 2 && (!profile.industry || x.parent_id === profile.industry)).map((x) => (
              <option key={x.id} value={x.id}>{x.name}</option>
            ))}
          </select>
          <input
            placeholder="关键词 (逗号分隔)"
            style={{ minWidth: 180 }}
            value={(profile.keywords ?? []).join(",")}
            onChange={(e) => setProfile({ ...profile, keywords: e.target.value.split(",").map((x) => x.trim()).filter(Boolean) })}
          />
          <input
            placeholder="竞争对手 (逗号分隔)"
            style={{ minWidth: 160 }}
            value={(profile.competitors ?? []).join(",")}
            onChange={(e) => setProfile({ ...profile, competitors: e.target.value.split(",").map((x) => x.trim()).filter(Boolean) })}
          />
          <input
            placeholder="目标市场 (逗号分隔)"
            style={{ minWidth: 140 }}
            value={(profile.geography ?? []).join(",")}
            onChange={(e) => setProfile({ ...profile, geography: e.target.value.split(",").map((x) => x.trim()).filter(Boolean) })}
          />
          <button onClick={saveProfile}>保存画像</button>
        </div>
        <p className="muted" style={{ marginTop: 6 }}>
          画像用于相关性评分 (行业30% / 主题25% / 竞品20% / 受众15% / 地域10%) 与内容自动打行业 tag。
        </p>
      </section>

      <section className="panel">
        <h2>情报信号 (Pipeline Signals)</h2>
        <p className="muted" style={{ marginTop: 0 }}>
          管线: {pipelines.filter((p) => p.enabled).length} 条 · 每个 record_type 一个 bus 主题 (intel.raw.*)
        </p>
        <table>
          <thead><tr><th>信号</th><th>来源</th><th>强度</th><th>重要性</th><th>时间</th></tr></thead>
          <tbody>
            {signals.map((s) => (
              <tr key={s.signal_id}>
                <td>{s.signal_type}</td>
                <td className="muted">{s.source_id}</td>
                <td>{s.strength.toFixed(2)}</td>
                <td>{s.importance.toFixed(2)}</td>
                <td className="muted">{s.observed_at.slice(0, 19).replace("T", " ")}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section className="panel">
        <h2>最近采集条目</h2>
        <div style={{ display: "flex", gap: 8, marginBottom: 8, alignItems: "center" }}>
          <select value={industryFilter} onChange={(e) => setIndustryFilter(e.target.value)}>
            <option value="">全部行业</option>
            {industries.map((x) => (
              <option key={x.id} value={x.id}>{x.name}</option>
            ))}
          </select>
          <label>
            <input
              type="checkbox"
              checked={relevanceMode}
              onChange={(e) => setRelevanceMode(e.target.checked)}
              disabled={!brandId}
            />{" "}
            按品牌相关性排序
          </label>
        </div>
        <table>
          <thead><tr><th>kind</th><th>信源</th><th>内容</th><th>行业</th><th>相关性</th><th>时间</th></tr></thead>
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
                  <td className="muted">
                    {(i.industries ?? []).map((ind) => {
                      const d = industries.find((x) => x.id === ind);
                      return d ? d.name : ind;
                    }).join(", ") || "-"}
                  </td>
                  <td className="muted">{i.relevance_score !== undefined ? i.relevance_score.toFixed(1) : "-"}</td>
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
