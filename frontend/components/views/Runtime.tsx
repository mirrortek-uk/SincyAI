"use client";

// 运行视图: 设备/Agent · 指标涨粉 · 内容流转追踪

import { useEffect, useState } from "react";

import {
  api,
  type AccountMetricAccount,
  type CapabilityItem,
  type DeviceItem,
  type PackageItem,
  type StrategyVersion,
  type TaskItem,
} from "@/lib/api";
import { Badge, Err, Loading, Spark, fmtDelta } from "@/components/ui";

// ---------- 📱 设备 / Agent ----------
export function DevicesView() {
  const [devices, setDevices] = useState<DeviceItem[] | null>(null);
  const [caps, setCaps] = useState<CapabilityItem[] | null>(null);
  const [tasks, setTasks] = useState<TaskItem[] | null>(null);
  const [code, setCode] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    Promise.all([api.listDevices(), api.listCapabilities("x"), api.listTasks()])
      .then(([d, c, t]) => { setDevices(d.items); setCaps(c.items); setTasks(t.items); })
      .catch((e) => setErr(String(e)));
  }, []);
  if (err) return <Err e={err} />;
  if (!devices || !caps || !tasks) return <Loading />;
  const waiting = tasks.filter((t) => t.state === "WAITING_USER");
  return (
    <section className="view active">
      <h1>📱 设备 / Agent</h1>
      <p className="lede">手机 agent 执行发布/回复等设备动作；人机验证一律转人工 (WAITING_USER)。</p>
      <div className="card">
        <h3>已配对设备 <span className="sp" />
          <button className="btn sm" onClick={async () => { try { const r = await api.createPairingCode(30); setCode(`${r.code} (有效期至 ${r.expires_at.slice(11, 19)})`); } catch (e) { setErr(String(e)); } }}>生成配对码</button>
        </h3>
        {code && <p className="sub">配对码: <b>{code}</b></p>}
        <table><tr><th>状态</th><th>名称</th><th>平台</th><th>Agent 版本</th><th>最后心跳</th></tr>
          {devices.map((d) => (
            <tr key={d.id}>
              <td>{d.state === "active" && d.last_seen && Date.now() - new Date(d.last_seen).getTime() < 120000
                ? <Badge kind="ok">● online</Badge> : <Badge kind="pend">{d.state}</Badge>}</td>
              <td>{d.name ?? d.id.slice(0, 8)}</td>
              <td>{d.platform}</td>
              <td className="sub">{d.agent_version ?? "?/?"}</td>
              <td className="sub">{d.last_seen ? d.last_seen.slice(11, 19) : "—"}</td>
            </tr>
          ))}
        </table>
      </div>
      <div className="card mt14">
        <h3>设备动作能力 (x 平台适配器)</h3>
        <div className="flowmini" style={{ gap: 8 }}>
          {caps.map((c) => (
            <span key={c.action} className="s" style={c.status === "verified" ? undefined : { borderColor: "var(--warn)", opacity: 0.8 }}>
              {c.action} {c.status === "verified" ? "✓" : `⚠ ${c.status}`}
            </span>
          ))}
        </div>
        <div className="sub mt14">read_followers / read_mentions 已随 0044/0045 签名启用 (Rust 解释器 + JNI read/capture 原语已实现); 加新动作 = 写 YAML 适配器 + capability 行。</div>
        {waiting.length > 0 && (
          <div className="alert b mt14">🛑 {waiting.length} 个任务 WAITING_USER (人机验证), 解锁手机后在「待你处理」页重试。</div>
        )}
      </div>
    </section>
  );
}

// ---------- 📈 指标 / 涨粉 ----------
export function AnalyticsView() {
  const [accounts, setAccounts] = useState<AccountMetricAccount[] | null>(null);
  const [strategies, setStrategies] = useState<Record<string, StrategyVersion | null>>({});
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    api.listAccountMetricsRecent().then(async (d) => {
      setAccounts(d.accounts);
      const emps = await api.listEmployees().catch(() => null);
      const map: Record<string, StrategyVersion | null> = {};
      if (emps) for (const e of emps.items) {
        try { const v = await api.listStrategyVersions(e.id); map[e.id] = v.items.find((x) => x.status === "ACTIVE") ?? null; }
        catch { map[e.id] = null; }
      }
      setStrategies(map);
    }).catch((e) => setErr(String(e)));
  }, []);
  if (err) return <Err e={err} />;
  if (!accounts) return <Loading />;
  return (
    <section className="view active">
      <h1>📈 指标 / 涨粉</h1>
      <p className="lede">运营目标是涨粉 —— 这里是全平台粉丝曲线、距 KPI 进度; Δ粉丝已作为 reward 回流学习回路 (patch-B)。</p>
      {accounts.length === 0 && (
        <div className="card"><p className="sub">还没有粉丝样本。设备 read_followers 任务已启用 (x.read_followers active), 设备接入后自动采样; 也可 POST /intel/account-metrics 手动灌入。</p></div>
      )}
      {accounts.map((a) => {
        const st = a.employee_id ? strategies[a.employee_id] : null;
        const target = st?.strategy.target_followers ?? null;
        const progress = target && a.followers ? Math.min(100, Math.round((a.followers / target) * 100)) : null;
        return (
          <div className="card mt14" key={`${a.platform}:${a.account_id}`}>
            <h3>全平台粉丝增长 <span className="sub">{a.platform} · @{a.account_id}</span>
              <span className="sp" />
              {a.delta_7d !== null && <Badge kind={a.delta_7d >= 0 ? "ok" : "bad"}>7日 {a.delta_7d >= 0 ? "+" : ""}{a.delta_7d}</Badge>}
            </h3>
            <div className="kpi">{a.followers?.toLocaleString() ?? "—"} <span className="sub" style={{ fontSize: 13 }}>粉丝 · 来源 {a.source} · {a.observed_at.slice(0, 16).replace("T", " ")}</span></div>
            <Spark points={a.series.map((s) => s.followers)} width={600} height={120} />
            <div className="sub">
              {target ? `目标 ${target.toLocaleString()} · 进度 ${progress}%` : "未设 target_followers KPI (在数字员工页策略里配置)"}
              {fmtDelta(a.delta_7d) && <> · 7 日 Δ {fmtDelta(a.delta_7d)}</>}
            </div>
            {progress !== null && (
              <div style={{ marginTop: 6, height: 8, borderRadius: 4, background: "var(--panel2)" }}>
                <div style={{ width: `${progress}%`, height: "100%", borderRadius: 4, background: "var(--emp)" }} />
              </div>
            )}
          </div>
        );
      })}
    </section>
  );
}

// ---------- 🔎 内容流转追踪 ----------
export function LineageView() {
  const [q, setQ] = useState("");
  const [pkgs, setPkgs] = useState<PackageItem[] | null>(null);
  const [tasks, setTasks] = useState<TaskItem[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    Promise.all([api.listPackages(), api.listTasks()])
      .then(([p, t]) => { setPkgs(p.items); setTasks(t.items); })
      .catch((e) => setErr(String(e)));
  }, []);
  if (err) return <Err e={err} />;
  if (!pkgs || !tasks) return <Loading />;
  const pkg = pkgs.find((p) => p.id.startsWith(q.trim()) || q.trim() === "") ??
    pkgs.find((p) => (p.constraints?.provenance as Record<string, unknown> | undefined)?.factory_brief === q.trim());
  const related = pkg ? tasks.filter((t) => t.package_id === pkg.id) : [];
  const briefId = pkg ? (pkg.constraints?.provenance as Record<string, unknown> | undefined)?.factory_brief : null;
  return (
    <section className="view active">
      <h1>🔎 内容流转追踪</h1>
      <p className="lede">追任一条内容从原始记录到指标的全程，高亮卡在哪一步。</p>
      <div className="card">
        <div className="row">
          <input type="text" placeholder="粘贴 package_id / brief_id" value={q} onChange={(e) => setQ(e.target.value)} style={{ flex: 1 }} />
        </div>
        {!q.trim() && <p className="sub mt14">输入 package_id (或 factory_brief id) 查看流转。</p>}
        {q.trim() && !pkg && <p className="sub mt14">未找到匹配的内容包。</p>}
        {pkg && (
          <>
            <div className="flowmini mt14" style={{ gap: 5 }}>
              <span className="s">signal ✓</span>→<span className="s">opportunity ✓</span>→
              <span className="s" style={{ borderColor: "var(--fac)" }}>brief {String(briefId ?? "—").slice(0, 10)} ✓</span>→
              <span className="s">plan/master/variants ✓</span>→<span className="s">gate ✓</span>→
              {pkg.approval_status === "pending"
                ? <span className="s" style={{ borderColor: "var(--warn)" }}>审批 待人工 ⏸</span>
                : <span className="s">审批 {pkg.approval_status} ✓</span>}→
              {related.length > 0
                ? <span className="s hot">发布任务 {related[0].state}</span>
                : <span className="s" style={{ opacity: 0.5 }}>发布</span>}→
              <span className="s" style={{ opacity: 0.5 }}>指标</span>
            </div>
            <div className="sub mt14">
              结局: 停在「{pkg.approval_status === "pending" ? "工序6 审批" : pkg.approval_status}」· package {pkg.id.slice(0, 12)} · {pkg.platform} · {pkg.created_at.slice(0, 16).replace("T", " ")}
              {related.length > 0 && <> · 设备任务 {related[0].id.slice(0, 8)} state={related[0].state}</>}
            </div>
            <div className="sub" style={{ marginTop: 8 }}>{pkg.text_preview.slice(0, 120)}</div>
          </>
        )}
      </div>
    </section>
  );
}
