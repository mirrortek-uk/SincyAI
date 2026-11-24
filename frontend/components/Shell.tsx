"use client";

// Console Redesign 壳: 左侧栏分组导航 + 顶部常驻数据流 rail + 主题切换。
// 路由 / 渲染新控制台 SPA; 旧路由页面渲染在 .main 内(兼容已有深链接); /login 裸渲染。

import { usePathname, useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";

import { api, getToken, setToken, type Me } from "@/lib/api";

export type ViewId =
  | "employees" | "factory" | "command" | "approvals" | "intel" | "signals"
  | "sources" | "devices" | "analytics" | "lineage" | "settings" | "arch" | "kill";

const NAV: { group: string; items: { v: ViewId; ic: string; label: string; core?: "emp" | "fac" }[] }[] = [
  { group: "核心 · 业务围绕这两个", items: [
    { v: "employees", ic: "🧑‍🚀", label: "数字员工", core: "emp" },
    { v: "factory", ic: "🏭", label: "内容工厂", core: "fac" },
  ]},
  { group: "指挥", items: [
    { v: "command", ic: "🎛️", label: "指挥台" },
    { v: "approvals", ic: "✅", label: "待你处理" },
  ]},
  { group: "数据流 · 情报", items: [
    { v: "intel", ic: "🌐", label: "世界情报" },
    { v: "signals", ic: "📡", label: "信号 / 机会" },
    { v: "sources", ic: "🩺", label: "信源健康 A/B/C" },
  ]},
  { group: "运行", items: [
    { v: "devices", ic: "📱", label: "设备 / Agent" },
    { v: "analytics", ic: "📈", label: "指标 / 涨粉" },
    { v: "lineage", ic: "🔎", label: "内容流转追踪" },
  ]},
  { group: "配置", items: [
    { v: "settings", ic: "⚙️", label: "设置" },
    { v: "arch", ic: "🧭", label: "架构 (as-built)" },
    { v: "kill", ic: "🛑", label: "急停开关" },
  ]},
];

export interface RailStats {
  sources: number | null;
  healthy: number | null;
  entities: number | null;
  signals: number | null;
  opportunities: number | null;
  factory: number | null;
  pending: number | null;
  publishedToday: number | null;
  running: number | null;
}

export function goView(v: ViewId) {
  if (typeof window !== "undefined") window.dispatchEvent(new CustomEvent("sincy:go", { detail: v }));
}

export function Shell({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  const router = useRouter();
  const [me, setMe] = useState<Me | null>(null);
  const [active, setActive] = useState<ViewId>("command");
  const [rail, setRail] = useState<RailStats>({
    sources: null, healthy: null, entities: null, signals: null,
    opportunities: null, factory: null, pending: null, publishedToday: null, running: null,
  });
  const [dark, setDark] = useState(false);

  // 主题: 渲染前由内联脚本应用, 这里只同步按钮
  useEffect(() => {
    setDark(document.documentElement.getAttribute("data-theme") === "dark");
  }, []);

  useEffect(() => {
    if (pathname === "/login") return;
    if (!getToken()) { router.push("/login"); return; }
    api.me().then(setMe).catch(() => { setToken(null); router.push("/login"); });
  }, [pathname, router]);

  // 控制台内视图切换
  useEffect(() => {
    const h = (e: Event) => setActive((e as CustomEvent<ViewId>).detail);
    window.addEventListener("sincy:go", h);
    const q = new URLSearchParams(window.location.search).get("v") as ViewId | null;
    if (q) setActive(q);
    return () => window.removeEventListener("sincy:go", h);
  }, []);

  // 顶部 rail 实时水位 (30s 轮询)
  useEffect(() => {
    if (pathname === "/login") return;
    let on = true;
    const load = async () => {
      try {
        const [status, stats, sig, briefs, pkgs, emps] = await Promise.all([
          api.intelSourceStatus().catch(() => null),
          api.intelStats().catch(() => null),
          api.listIntelSignals(undefined, 200).catch(() => null),
          api.listFactoryBriefs().catch(() => null),
          api.listPackages().catch(() => null),
          api.listEmployees().catch(() => null),
        ]);
        if (!on) return;
        const today = new Date().toISOString().slice(0, 10);
        setRail({
          sources: status ? status.sources.length : null,
          healthy: status ? status.sources.filter((s) => s.health === "healthy").length : null,
          entities: stats ? stats.total_items : null,
          signals: sig ? sig.items.length : null,
          opportunities: stats ? stats.opportunities : null,
          factory: briefs ? briefs.briefs.filter((b) => b.status === "draft" || b.status === "running").length : null,
          pending: pkgs ? pkgs.items.filter((p) => p.approval_status === "pending").length : null,
          publishedToday: pkgs
            ? pkgs.items.filter((p) => p.approval_status !== "pending" && p.approval_status !== "rejected" && p.created_at.slice(0, 10) === today).length
            : null,
          running: emps ? emps.items.filter((e) => e.status === "RUNNING").length : null,
        });
      } catch { /* 下轮再试 */ }
    };
    load();
    const t = setInterval(load, 30000);
    return () => { on = false; clearInterval(t); };
  }, [pathname]);

  if (pathname === "/login") return <>{children}</>;

  const nav = (v: ViewId, core?: "emp" | "fac") => (
    <div
      key={v}
      className={`nav${core ? ` core-${core}` : ""}${pathname === "/" && active === v ? " active" : ""}`}
      onClick={() => {
        if (pathname !== "/") router.push(`/?v=${v}`);
        else goView(v);
      }}
    >
      <span className="ic">{NAV.flatMap((g) => g.items).find((i) => i.v === v)?.ic}</span>
      {NAV.flatMap((g) => g.items).find((i) => i.v === v)?.label}
    </div>
  );

  const railItem = (n: string, v: number | null, vw: ViewId, cls?: string) => (
    <div className={`rstage ${cls ?? ""}`} onClick={() => (pathname !== "/" ? router.push(`/?v=${vw}`) : goView(vw))}>
      <span className="n">{n}</span>
      <span className="v">{v === null ? "—" : v}</span>
    </div>
  );

  return (
    <div className="app">
      <aside className="side">
        <div className="brand">Sincy<span className="dot">·</span>Console</div>
        {NAV.map((g) => (
          <div key={g.group}>
            <div className="navgroup">{g.group}</div>
            {g.items.map((i) => nav(i.v, i.core))}
          </div>
        ))}
      </aside>
      <div className="top">
        <div className="who">
          <span className="pill">机构 {me ? me.name : "…"}</span>
          <span>{me?.role ?? ""}</span>
        </div>
        <div className="rail">
          <div className="railwrap">
            {railItem("信源", rail.sources, "sources")}<span className="rarrow">→</span>
            {railItem("healthy", rail.healthy, "sources")}<span className="rarrow">→</span>
            {railItem("实体", rail.entities, "intel")}<span className="rarrow">→</span>
            {railItem("信号", rail.signals, "signals")}<span className="rarrow">→</span>
            {railItem("机会", rail.opportunities, "signals")}<span className="rarrow">⇒</span>
            {railItem("工厂", rail.factory, "factory", "fac")}<span className="rarrow">→</span>
            {railItem("待审", rail.pending, "approvals", "fac")}<span className="rarrow">→</span>
            {railItem("发布", rail.publishedToday, "analytics")}<span className="rarrow">↺</span>
            {railItem("学习", rail.running, "employees", "emp")}
          </div>
        </div>
        <button
          className="themebtn"
          onClick={() => {
            const d = document.documentElement.getAttribute("data-theme") === "dark";
            if (d) document.documentElement.removeAttribute("data-theme");
            else document.documentElement.setAttribute("data-theme", "dark");
            try { localStorage.setItem("sincy-theme", d ? "light" : "dark"); } catch { /* ignore */ }
            setDark(!d);
          }}
        >
          {dark ? "☀️ 淡色" : "🌙 暗色"}
        </button>
        <button className="btn fac sm" onClick={() => (pathname !== "/" ? router.push("/?v=factory&new=1") : goView("factory"))}>
          + 新建 Brief
        </button>
      </div>
      <main className="main">{children}</main>
    </div>
  );
}
