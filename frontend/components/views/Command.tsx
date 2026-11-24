"use client";

// 🎛️ 指挥台: 两个业务中心全景 + 数据流水位 + 需要处理的事 + 健康告警

import { useEffect, useState } from "react";

import { api, type EmployeeItem, type FactoryBrief, type IntelSourceStatusResp, type IntelStats, type PackageItem, type TaskItem } from "@/lib/api";
import { goView } from "@/components/Shell";
import { Badge, Err, healthBadge, Loading } from "@/components/ui";

export function CommandView() {
  const [emps, setEmps] = useState<EmployeeItem[] | null>(null);
  const [briefs, setBriefs] = useState<FactoryBrief[] | null>(null);
  const [pkgs, setPkgs] = useState<PackageItem[] | null>(null);
  const [tasks, setTasks] = useState<TaskItem[] | null>(null);
  const [srcs, setSrcs] = useState<IntelSourceStatusResp | null>(null);
  const [stats, setStats] = useState<IntelStats | null>(null);
  const [cyclesToday, setCyclesToday] = useState<number | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let on = true;
    (async () => {
      try {
        const [e, b, p, t, s, st] = await Promise.all([
          api.listEmployees(), api.listFactoryBriefs(), api.listPackages(), api.listTasks(), api.intelSourceStatus(), api.intelStats(),
        ]);
        if (!on) return;
        setEmps(e.items); setBriefs(b.briefs); setPkgs(p.items); setTasks(t.items); setSrcs(s); setStats(st);
        const today = new Date().toISOString().slice(0, 10);
        let n = 0;
        for (const emp of e.items) {
          try {
            const c = await api.listCycles(emp.id, 50);
            n += c.items.filter((x) => x.created_at.slice(0, 10) === today).length;
          } catch { /* 单个员工失败不影响总览 */ }
        }
        if (on) setCyclesToday(n);
      } catch (ex) { if (on) setErr(String(ex)); }
    })();
    return () => { on = false; };
  }, []);

  if (err) return <Err e={err} />;
  if (!emps || !briefs || !pkgs || !tasks || !srcs || !stats) return <Loading />;

  const running = emps.filter((e) => e.status === "RUNNING");
  const inflight = briefs.filter((b) => b.status === "draft" || b.status === "running").length;
  const pending = pkgs.filter((p) => p.approval_status === "pending");
  const today = new Date().toISOString().slice(0, 10);
  const published = pkgs.filter((p) => p.approval_status !== "pending" && p.approval_status !== "rejected" && p.created_at.slice(0, 10) === today).length;
  const blocked = briefs.filter((b) => b.status === "failed").length;
  const waitingUser = tasks.filter((t) => t.state === "WAITING_USER");
  const notCfg = srcs.sources.filter((s) => s.health === "not_configured");
  const unhealthy = srcs.sources.filter((s) => s.health === "degraded" || s.health === "failed");

  return (
    <section className="view active">
      <h1>🎛️ 指挥台</h1>
      <p className="lede">两个中心的全景 + 数据流当前水位 + 需要你处理的事。</p>

      <div className="grid g2">
        <div className="card" style={{ borderColor: "var(--emp)" }}>
          <h3><span className="tag t-emp">核心</span> 数字员工</h3>
          <div className="row">
            <div><div className="kpi" style={{ color: "var(--emp)" }}>{running.length}</div><div className="sub">运营中</div></div>
            <div className="sp" />
            <div style={{ textAlign: "right" }}>
              <div><b>{cyclesToday ?? "—"}</b> 今日周期</div>
              <div className="sub">{emps.length} 员工</div>
            </div>
          </div>
          <button className="btn emp sm mt14" onClick={() => goView("employees")}>进入数字员工 →</button>
        </div>
        <div className="card" style={{ borderColor: "var(--fac)" }}>
          <h3><span className="tag t-fac">核心</span> 内容工厂</h3>
          <div className="row">
            <div><div className="kpi" style={{ color: "var(--fac)" }}>{inflight}</div><div className="sub">在流转</div></div>
            <div className="sp" />
            <div style={{ textAlign: "right" }}>
              <div><b>{pending.length}</b> 待审 · <b>{published}</b> 今日发布</div>
              <div className="sub">{blocked} 被拦下</div>
            </div>
          </div>
          <button className="btn fac sm mt14" onClick={() => goView("factory")}>进入内容工厂 →</button>
        </div>
      </div>

      <div className="card mt14">
        <h3>📊 数据流水位 (点节点跳转)</h3>
        <div className="flowmini mt14" style={{ fontSize: "12.5px", gap: "6px" }}>
          <span className="s">信源 {srcs.sources.length}</span>→
          <span className="s">healthy {srcs.sources.filter((s) => s.health === "healthy").length}</span>→
          <span className="s">实体 {stats.total_items}</span>→
          <span className="s hot">机会 {stats.opportunities}</span>
          <span className="s hot">待审 {pending.length}</span>⇒
          <span className="s" style={{ borderColor: "var(--fac)" }}>工厂 {inflight}</span>→
          <span className="s" style={{ borderColor: "var(--warn)" }}>待审 {pending.length}</span>→
          <span className="s">今日发布 {published}</span>↺
          <span className="s" style={{ borderColor: "var(--emp)" }}>学习 {running.length} 跑</span>
        </div>
      </div>

      <div className="grid g2 mt14">
        <div className="card">
          <h3>🔔 需要你处理</h3>
          {pending.length > 0 && (
            <div className="alert w">⏳ {pending.length} 条内容待审批 <span className="sp" />
              <button className="btn sm" onClick={() => goView("approvals")}>去处理</button></div>
          )}
          {waitingUser.length > 0 && (
            <div className="alert b">🛑 {waitingUser.length} 个设备任务 WAITING_USER (人机验证) — 需手机解锁 <span className="sp" />
              <button className="btn ghost sm" onClick={() => goView("devices")}>查看</button></div>
          )}
          {notCfg.length > 0 && (
            <div className="alert w">⚠ {notCfg.length} 个信源 not_configured (缺 key/自托管) <span className="sp" />
              <button className="btn ghost sm" onClick={() => goView("sources")}>查看</button></div>
          )}
          {pending.length === 0 && waitingUser.length === 0 && notCfg.length === 0 && (
            <p className="sub">暂无待处理事项 🎉</p>
          )}
        </div>
        <div className="card">
          <h3>⚠ 健康</h3>
          {unhealthy.length === 0 && <p className="sub">信源全部健康 ✅</p>}
          {unhealthy.slice(0, 6).map((s) => (
            <div className="field" key={s.source_id}>
              <label>{s.name} <span className="hint">{s.connector ?? s.class}</span></label>
              {healthBadge(s.health)}
            </div>
          ))}
          <div className="field mb0"><label>LLM provider</label><Badge kind="ok">见 /ai/generate</Badge></div>
        </div>
      </div>
    </section>
  );
}
