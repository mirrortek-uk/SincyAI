"use client";

// Sincy Console Redesign 首页: 单页视图切换 (侧栏/rail 通过 sincy:go 事件驱动)

import { useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState } from "react";

import type { ViewId } from "@/components/Shell";
import { ApprovalsView, IntelView, SignalsView, SourcesView } from "@/components/views/Data";
import { CommandView } from "@/components/views/Command";
import { EmployeesView } from "@/components/views/Employees";
import { FactoryView } from "@/components/views/Factory";
import { AnalyticsView, DevicesView, LineageView } from "@/components/views/Runtime";
import { ArchView, KillView, SettingsView } from "@/components/views/Config";
import { KpiView } from "@/components/views/Kpi";
import { VoiceView } from "@/components/views/Voice";

function ConsoleInner() {
  const search = useSearchParams();
  const [view, setView] = useState<ViewId>((search.get("v") as ViewId) || "command");
  const [openNew, setOpenNew] = useState(search.get("new") === "1");

  useEffect(() => {
    const h = (e: Event) => setView((e as CustomEvent<ViewId>).detail);
    window.addEventListener("sincy:go", h);
    return () => window.removeEventListener("sincy:go", h);
  }, []);

  return (
    <>
      <div className={view === "command" ? "view active" : "view"}><CommandView /></div>
      <div className={view === "employees" ? "view active" : "view"}><EmployeesView /></div>
      <div className={view === "factory" ? "view active" : "view"}>
        <FactoryView openNew={openNew && view === "factory"} />
      </div>
      <div className={view === "approvals" ? "view active" : "view"}><ApprovalsView /></div>
      <div className={view === "intel" ? "view active" : "view"}><IntelView /></div>
      <div className={view === "signals" ? "view active" : "view"}><SignalsView /></div>
      <div className={view === "sources" ? "view active" : "view"}><SourcesView /></div>
      <div className={view === "devices" ? "view active" : "view"}><DevicesView /></div>
      <div className={view === "analytics" ? "view active" : "view"}><AnalyticsView /></div>
      <div className={view === "lineage" ? "view active" : "view"}><LineageView /></div>
      <div className={view === "settings" ? "view active" : "view"}><SettingsView /></div>
      <div className={view === "arch" ? "view active" : "view"}><ArchView /></div>
      <div className={view === "kill" ? "view active" : "view"}><KillView /></div>
      <div className={view === "kpi" ? "view active" : "view"}><KpiView /></div>
      <div className={view === "voice" ? "view active" : "view"}><VoiceView /></div>
    </>
  );
}

export default function ConsolePage() {
  return (
    <Suspense fallback={<p className="sub">加载中…</p>}>
      <ConsoleInner />
    </Suspense>
  );
}
