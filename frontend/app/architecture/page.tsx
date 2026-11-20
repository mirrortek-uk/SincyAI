"use client";

import { useEffect, useMemo, useState } from "react";
import { api, getToken, type ArchEdge, type Architecture } from "@/lib/api";

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

  if (error) return <main><h1>架构图</h1><p style={{ color: "#ef4444" }}>{error}</p></main>;
  if (!arch) return <main><h1>架构图</h1><p className="muted">加载中…</p></main>;

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
    <main>
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
