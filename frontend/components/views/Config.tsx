"use client";

// 配置视图: 设置 / 架构(as-built, 内嵌既有架构页) / 急停开关

import { useEffect, useState } from "react";

import { api, type ArchSkillDomain, type KillSwitch, type ReviewRule } from "@/lib/api";
import ArchitecturePage from "@/app/architecture/page";
import { Badge, Err, Loading } from "@/components/ui";

// ---------- ⚙️ 设置 ----------
export function SettingsView() {
  const [rules, setRules] = useState<ReviewRule[] | null>(null);
  const [domains, setDomains] = useState<{ domains: ArchSkillDomain[]; total_skills: number } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [edit, setEdit] = useState<Record<string, { weighted_limit: string; on_over_limit: string }>>({});
  useEffect(() => {
    Promise.all([api.listReviewRules(), api.listSkillDomains()])
      .then(([r, d]) => { setRules(r.items); setDomains(d); })
      .catch((e) => setErr(String(e)));
  }, []);
  if (err) return <Err e={err} />;
  if (!rules || !domains) return <Loading />;
  const save = async (platform: string) => {
    const e = edit[platform];
    if (!e) return;
    try {
      await api.upsertReviewRule(platform, { weighted_limit: Number(e.weighted_limit) || 40000, on_over_limit: e.on_over_limit });
      setMsg(`平台 ${platform} 审核门已保存 ✅`);
    } catch (ex) { setMsg(String(ex)); }
  };
  return (
    <section className="view active">
      <h1>⚙️ 设置</h1>
      <p className="lede">集中配置。员工/工厂的局部配置在各自页内。</p>
      {msg && <p className="sub">{msg}</p>}
      <div className="grid g2">
        <div className="card">
          <h3>平台审核门</h3>
          <table><tr><th>平台</th><th>加权字数上限</th><th>超限</th><th></th></tr>
            {rules.map((r) => {
              const e = edit[r.platform] ?? { weighted_limit: String(r.weighted_limit), on_over_limit: r.on_over_limit };
              return (
                <tr key={r.platform}>
                  <td>{r.platform}</td>
                  <td><input type="number" value={e.weighted_limit} style={{ width: 90 }}
                    onChange={(ev) => setEdit({ ...edit, [r.platform]: { ...e, weighted_limit: ev.target.value } })} /></td>
                  <td><select value={e.on_over_limit}
                    onChange={(ev) => setEdit({ ...edit, [r.platform]: { ...e, on_over_limit: ev.target.value } })}>
                    <option value="reject">reject</option><option value="thread">thread</option>
                  </select></td>
                  <td><button className="btn sm" onClick={() => save(r.platform)}>保存</button></td>
                </tr>
              );
            })}
          </table>
        </div>
        <div className="card">
          <h3>调度 & 测量 <span className="sub">(环境变量, 重启生效)</span></h3>
          <div className="field"><label>员工调度间隔</label><span className="sub">SINCY_EMPLOYEE_SCHEDULER_INTERVAL_SECS · 默认 60s</span></div>
          <div className="field"><label>延迟测量</label><span className="sub">SINCY_MEASURE_DELAY_SECS · 默认 6h</span></div>
          <div className="field"><label>粉丝采样间隔</label><span className="sub">SINCY_ACCOUNT_METRIC_INTERVAL_SECS · 默认 6h</span></div>
          <div className="field mb0"><label>互动子循环间隔</label><span className="sub">SINCY_INTERACTION_INTERVAL_SECS · 默认 2h</span></div>
        </div>
      </div>
      <div className="grid g2 mt14">
        <div className="card">
          <h3>Providers / Keys</h3>
          <div className="field"><label>LLM provider</label><Badge kind="ok">见 /ai/generate (SINCY_LLM_*)</Badge></div>
          <div className="field"><label>Firecrawl key</label><Badge kind="pend">SINCY_FIRECRAWL_API_KEY 未配 = 本地正则剥页</Badge></div>
          <div className="field mb0"><label>媒体 providers</label><span className="sub">SINCY_MEDIA_PROVIDERS (工序4)</span></div>
        </div>
        <div className="card">
          <h3>技能 catalog</h3>
          <div className="sub">{domains.total_skills} skills / {domains.domains.length} 域 · 可 POST /api/v1/skills 动态注册新版本</div>
          <div className="flowmini mt14" style={{ gap: 6 }}>
            {domains.domains.map((d) => <span key={d.id} className="s">{d.title_zh ?? d.id} {d.count}</span>)}
          </div>
        </div>
      </div>
    </section>
  );
}

// ---------- 🧭 架构 (as-built) ----------
export function ArchView() {
  return (
    <section className="view active">
      <ArchitecturePage />
    </section>
  );
}

// ---------- 🛑 急停开关 ----------
export function KillView() {
  const [items, setItems] = useState<KillSwitch[] | null>(null);
  const [reason, setReason] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const load = () => api.listKillSwitches().then((d) => setItems(d.items)).catch((e) => setErr(String(e)));
  useEffect(() => { load(); }, []);
  if (err) return <Err e={err} />;
  if (!items) return <Loading />;
  const active = items.filter((k) => k.active);
  return (
    <section className="view active">
      <h1>🛑 急停开关</h1>
      <p className="lede">一键停止自主发布/设备动作。当前 {active.length} 个急停激活。</p>
      {msg && <p className="sub">{msg}</p>}
      <div className="card">
        <div className="row">
          <div><b>新建急停</b><div className="sub">scope: agency(全局) / employee / device</div></div>
          <span className="sp" />
          <select id="ks-scope" defaultValue="agency"><option value="agency">agency 全局</option><option value="employee">employee</option><option value="device">device</option></select>
          <input type="text" placeholder="原因" value={reason} onChange={(e) => setReason(e.target.value)} style={{ width: 200 }} />
          <button className="btn" onClick={async () => {
            try {
              const scope = (document.getElementById("ks-scope") as HTMLSelectElement).value;
              await api.createKillSwitch({ scope, reason: reason || "manual" });
              setMsg("急停已激活 🛑"); setReason(""); load();
            } catch (e) { setMsg(String(e)); }
          }}>激活急停</button>
        </div>
      </div>
      <div className="card mt14">
        <h3>急停列表</h3>
        <table><tr><th>状态</th><th>scope</th><th>目标</th><th>原因</th><th>操作</th></tr>
          {items.map((k) => (
            <tr key={k.id}>
              <td>{k.active ? <Badge kind="bad">ACTIVE</Badge> : <Badge kind="ok">已解除</Badge>}</td>
              <td>{k.scope}</td>
              <td className="sub">{k.target_id ?? "—"}</td>
              <td className="sub">{k.reason}</td>
              <td>{k.active && <button className="btn ghost sm" onClick={async () => { await api.deactivateKillSwitch(k.id); load(); }}>解除</button>}</td>
            </tr>
          ))}
          {items.length === 0 && <tr><td colSpan={5} className="sub">暂无急停记录</td></tr>}
        </table>
      </div>
    </section>
  );
}
