"use client";

import { useEffect, useMemo, useState } from "react";
import { api, getToken, type ArchEdge, type Architecture } from "@/lib/api";

// 运营周期 10 步的中文名 (与后端 cycle_steps 顺序一致, 供动画高亮)
const STEP_ZH: Record<string, string> = {
  observe: "感知", understand: "理解", decide: "决策", plan: "计划", create: "创作",
  review: "审核", execute: "执行", measure: "衡量", learn: "学习", adapt: "适应",
};

// 架构图页: 从后端 GET /api/v1/architecture 取结构化模型,
// 在 console 内用 SVG 渲染分层架构图 + 边标注 + 关键数据流。

const LAYER_W = 300;
const LAYER_GAP = 110;
const NODE_W = 270;
const NODE_H = 78;
const NODE_GAP = 26;
const PAD = 30;
const HEADER_H = 70;

const KIND_COLOR: Record<string, string> = {
  http: "#4c8dff",
  ws: "#22c55e",
  internal: "#a78bfa",
  data: "#f59e0b",
  external: "#ef4444",
  inproc: "#22d3ee",
  signal: "#f472b6",
  sql: "#a3e635",
};

interface Pos {
  x: number;
  y: number;
  cx: number;
  cy: number;
}

export default function ArchitecturePage() {
  const [arch, setArch] = useState<Architecture | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [hover, setHover] = useState<string | null>(null);

  useEffect(() => {
    if (!getToken()) return;
    api
      .getArchitecture()
      .then(setArch)
      .catch((e) => setError(String(e)));
  }, []);

  // 计算每个节点的位置: 层 = 列, 节点在列内纵向排布
  const pos = useMemo(() => {
    const map = new Map<string, Pos>();
    if (!arch) return map;
    arch.layers.forEach((layer, li) => {
      const x = PAD + li * (LAYER_W + LAYER_GAP);
      layer.nodes.forEach((n, ni) => {
        const y = HEADER_H + ni * (NODE_H + NODE_GAP);
        map.set(n.id, { x, y, cx: x + NODE_W / 2, cy: y + NODE_H / 2 });
      });
    });
    return map;
  }, [arch]);

  if (error) return <main className="arch-root"><h1>架构图</h1><p style={{ color: "#ef4444" }}>{error}</p></main>;
  if (!arch) return <main className="arch-root"><h1>架构图</h1><p className="muted">加载中…</p></main>;

  const width = PAD * 2 + arch.layers.length * LAYER_W + (arch.layers.length - 1) * LAYER_GAP;
  const height =
    HEADER_H + Math.max(...arch.layers.map((l) => l.nodes.length)) * (NODE_H + NODE_GAP) + PAD;

  const edgePath = (e: ArchEdge) => {
    const a = pos.get(e.from);
    const b = pos.get(e.to);
    if (!a || !b) return "";
    // 同层: 下边缘 → 上边缘; 跨层: 右边缘 → 左边缘
    if (a.cx === b.cx && a.cy !== b.cy) {
      const x = a.cx;
      const y1 = a.cy > b.cy ? a.y : a.y + NODE_H;
      const y2 = a.cy > b.cy ? b.y + NODE_H : b.y;
      return `M ${x} ${y1} C ${x + 40} ${y1}, ${x + 40} ${y2}, ${x} ${y2}`;
    }
    const x1 = a.cx < b.cx ? a.x + NODE_W : a.x;
    const x2 = a.cx < b.cx ? b.x : b.x + NODE_W;
    const y1 = a.cy;
    const y2 = b.cy;
    const mx = (x1 + x2) / 2;
    return `M ${x1} ${y1} C ${mx} ${y1}, ${mx} ${y2}, ${x2} ${y2}`;
  };

  const edgeMid = (e: ArchEdge) => {
    const a = pos.get(e.from);
    const b = pos.get(e.to);
    if (!a || !b) return { x: 0, y: 0 };
    return { x: (a.cx + b.cx) / 2, y: (a.cy + b.cy) / 2 };
  };

  return (
    <main className="arch-root">
      <h1>{arch.title}</h1>
      <p className="muted">
        {arch.service} v{arch.version} · 模型生成时间 {arch.generated_at.slice(0, 19).replace("T", " ")} ·
        数据来自 <code>GET /api/v1/architecture</code>, 节点/边变更请同步修改 cloud-api 的 architecture.rs
      </p>

      <div style={{ overflowX: "auto", border: "1px solid var(--border)", borderRadius: 8 }}>
        <svg width={width} height={height} style={{ display: "block", minWidth: width }}>
          <defs>
            {Object.entries(KIND_COLOR).map(([k, c]) => (
              <marker key={k} id={`arrow-${k}`} markerWidth="9" markerHeight="9" refX="8" refY="4.5" orient="auto">
                <path d="M0,0 L9,4.5 L0,9 z" fill={c} />
              </marker>
            ))}
          </defs>

          {/* 层标题 */}
          {arch.layers.map((layer, li) => (
            <text
              key={layer.id}
              x={PAD + li * (LAYER_W + LAYER_GAP) + LAYER_W / 2}
              y={34}
              textAnchor="middle"
              style={{ fontSize: 16, fontWeight: 700, fill: "var(--text)" }}
            >
              {layer.title}
            </text>
          ))}

          {/* planned 层虚线框 */}
          {arch.layers.map((layer, li) => {
            const h = Math.max(...arch.layers.map((l) => l.nodes.length)) * (NODE_H + NODE_GAP) + 10;
            return layer.planned ? (
              <rect
                key={"pl-" + layer.id}
                x={PAD + li * (LAYER_W + LAYER_GAP) - 8}
                y={HEADER_H - 26}
                width={LAYER_W + 16}
                height={h}
                rx={12}
                fill="none"
                stroke="var(--border)"
                strokeDasharray="6 4"
                opacity={0.7}
              />
            ) : null;
          })}

          {/* 边 */}
          {arch.edges.map((e, i) => {
            const c = KIND_COLOR[e.kind] ?? "#888";
            const dim = hover && hover !== e.from && hover !== e.to;
            return (
              <g key={i} opacity={dim ? 0.15 : 1}>
                <path
                  d={edgePath(e)}
                  fill="none"
                  stroke={c}
                  strokeWidth={hover === e.from || hover === e.to ? 2.6 : 1.4}
                  strokeDasharray={e.kind === "external" ? "6 4" : undefined}
                  markerEnd={`url(#arrow-${e.kind})`}
                />
                <text
                  x={edgeMid(e).x}
                  y={edgeMid(e).y - 4}
                  textAnchor="middle"
                  style={{
                    fontSize: 10.5,
                    fill: c,
                    paintOrder: "stroke",
                    stroke: "#0f1115",
                    strokeWidth: 4,
                  }}
                >
                  {e.label}
                </text>
              </g>
            );
          })}

          {/* 节点 */}
          {arch.layers.map((layer, li) =>
            layer.nodes.map((n) => {
              const p = pos.get(n.id);
              if (!p) return null;
              const active = hover === n.id;
              return (
                <g
                  key={n.id}
                  onMouseEnter={() => setHover(n.id)}
                  onMouseLeave={() => setHover(null)}
                  style={{ cursor: "default" }}
                >
                  <rect
                    x={p.x}
                    y={p.y}
                    width={NODE_W}
                    height={NODE_H}
                    rx={10}
                    fill={active ? "#1d2a44" : "var(--panel)"}
                    stroke={active ? "var(--accent)" : "var(--border)"}
                    strokeWidth={active ? 2 : 1}
                  />
                  <text x={p.x + 12} y={p.y + 22} style={{ fontSize: 13.5, fontWeight: 700, fill: "var(--text)" }}>
                    {n.title}
                  </text>
                  <text x={p.x + 12} y={p.y + 40} style={{ fontSize: 10.5, fill: "var(--accent)" }}>
                    {n.tech.length > 42 ? n.tech.slice(0, 42) + "…" : n.tech}
                  </text>
                  <text x={p.x + 12} y={p.y + 58} style={{ fontSize: 10, fill: "var(--muted)" }}>
                    {n.desc.length > 52 ? n.desc.slice(0, 52) + "…" : n.desc}
                  </text>
                  <title>{`${n.title}\n${n.tech}\n${n.desc}`}</title>
                </g>
              );
            }),
          )}
        </svg>
      </div>

      {/* 图例 */}
      <div style={{ display: "flex", gap: 16, flexWrap: "wrap", marginTop: 10 }}>
        {Object.entries(arch.edge_kinds).map(([k, label]) => (
          <span key={k} className="muted" style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
            <span style={{ width: 22, height: 3, background: KIND_COLOR[k] ?? "#888", display: "inline-block" }} />
            {label}
          </span>
        ))}
      </div>

      {/* 数字员工: 6 核心 Agent × 10 步运营周期 (runtime 高亮已跑/当前步骤, data-* 供后续动画) */}
      {arch.agents && arch.cycle_steps && (
        <section className="panel" style={{ marginTop: 16 }}>
          <h2>数字员工 · {arch.agents.length} 核心 Agent × {arch.cycle_steps.length} 步运营周期</h2>
          {arch.runtime && (
            <p className="muted" style={{ marginTop: 0 }}>
              最近周期 <code data-runtime="cycle">{arch.runtime.cycle_id.slice(0, 8)}</code> · 员工{" "}
              <strong>{arch.runtime.employee_name}</strong> · {arch.runtime.state} · {arch.runtime.trigger} ·{" "}
              {arch.runtime.created_at.slice(0, 19).replace("T", " ")}
            </p>
          )}
          <div style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "center", margin: "8px 0" }}>
            {arch.cycle_steps.map((s, i) => {
              const rt = arch.runtime?.steps.find((x) => x.step === s);
              const agent = arch.agents.find((a) => a.steps.includes(s));
              const done = !!rt;
              return (
                <span key={s} style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
                  <span
                    data-step={s}
                    data-agent={agent?.id ?? ""}
                    data-state={rt ? rt.state : "idle"}
                    title={rt ? `${agent?.name_zh ?? ""} · agent=${rt.agent} · state=${rt.state}` : `${agent?.name_zh ?? ""} · 未运行`}
                    style={{
                      padding: "4px 10px",
                      borderRadius: 999,
                      border: `1px solid ${done ? "var(--accent)" : "var(--border)"}`,
                      background: done ? "rgba(76,141,255,0.15)" : "transparent",
                      fontSize: 12.5,
                      fontWeight: done ? 700 : 400,
                    }}
                  >
                    {i + 1}. {STEP_ZH[s] ?? s}
                    <span className="muted" style={{ fontSize: 10.5 }}> · {agent?.name_zh ?? "—"}</span>
                  </span>
                  {i < arch.cycle_steps.length - 1 && <span className="muted">→</span>}
                </span>
              );
            })}
          </div>
          {arch.runtime && arch.runtime.steps.length > 0 && (
            <details style={{ marginTop: 6 }}>
              <summary className="muted">本周期各步骤实际调用的 agent / skills</summary>
              <table style={{ marginTop: 6 }}>
                <thead><tr><th>步骤</th><th>agent</th><th>state</th><th>skills</th></tr></thead>
                <tbody>
                  {arch.runtime.steps.map((st, i) => (
                    <tr key={i}>
                      <td data-step={st.step}>{st.step}</td>
                      <td data-agent={st.agent}>{st.agent}</td>
                      <td>{st.state}</td>
                      <td className="muted" style={{ fontSize: 11 }}>
                        {Array.isArray(st.skills) ? (st.skills as string[]).join(", ") : st.skill ? JSON.stringify(st.skill) : "—"}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </details>
          )}
          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(240px, 1fr))", gap: 10, marginTop: 12 }}>
            {arch.agents.map((a) => (
              <div key={a.id} data-agent={a.id} style={{ border: "1px solid var(--border)", borderRadius: 8, padding: 10 }}>
                <strong>{a.name_zh}</strong> <span className="muted" style={{ fontSize: 11 }}>({a.id})</span>
                <p className="muted" style={{ margin: "4px 0", fontSize: 12 }}>{a.role}</p>
                <p className="muted" style={{ margin: 0, fontSize: 11 }}>
                  步骤: {a.steps.map((s) => STEP_ZH[s] ?? s).join(" → ") || "协调"}
                  {a.domains.length > 0 && <> · skill 域: {a.domains.join(", ")}</>}
                </p>
              </div>
            ))}
          </div>
        </section>
      )}

      {/* Skill 生态: 9 域 */}
      {arch.skill_domains && (
        <section className="panel" style={{ marginTop: 16 }}>
          <h2>Skill 生态 · {arch.skill_domains.total_skills} 个 skills / {arch.skill_domains.domains.length} 域</h2>
          <p className="muted" style={{ marginTop: 0 }}>
            Agent 是角色/决策主体, Skill 是可复用、可版本化的能力模块 (skills / skill_versions / employee_skill_bindings,
            初始 catalog 见迁移 0021, 可经 <code>POST /api/v1/skills</code> 动态注册新版本)
          </p>
          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(260px, 1fr))", gap: 10 }}>
            {arch.skill_domains.domains.map((d) => (
              <div key={d.id} data-skill-domain={d.id} style={{ border: "1px solid var(--border)", borderRadius: 8, padding: 10 }}>
                <strong>{d.title_zh ?? d.id}</strong>{" "}
                <span className="muted" style={{ fontSize: 11 }}>({d.id}) · {d.count} 个</span>
                <div style={{ display: "flex", flexWrap: "wrap", gap: 4, marginTop: 6 }}>
                  {d.skills.slice(0, 14).map((s) => (
                    <code key={s} data-skill={s} style={{ fontSize: 10.5, padding: "1px 6px", border: "1px solid var(--border)", borderRadius: 999 }}>{s}</code>
                  ))}
                  {d.skills.length > 14 && <span className="muted" style={{ fontSize: 10.5 }}>+{d.skills.length - 14}</span>}
                </div>
              </div>
            ))}
          </div>
        </section>
      )}

      {/* 关键数据流 */}
      <section className="panel" style={{ marginTop: 16 }}>
        <h2>关键数据流</h2>
        {arch.flows.map((f) => (
          <div key={f.id} style={{ marginBottom: 10 }}>
            <strong>{f.id} · {f.title}</strong>
            <p className="muted" style={{ margin: "4px 0 0" }}>{f.steps}</p>
          </div>
        ))}
      </section>
    </main>
  );
}
