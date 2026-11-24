"use client";

// 共享小组件: 徽章 / 火花图 / 加载态

export function Badge({ kind, children }: { kind: "ok" | "warn" | "bad" | "pend" | "run"; children: React.ReactNode }) {
  return <span className={`badge b-${kind}`}>{children}</span>;
}

export function healthBadge(h: string) {
  if (h === "healthy") return <Badge kind="ok">healthy</Badge>;
  if (h === "degraded") return <Badge kind="warn">degraded</Badge>;
  if (h === "failed") return <Badge kind="bad">failed</Badge>;
  return <Badge kind="pend">{h}</Badge>;
}

export function Spark({
  points,
  width = 260,
  height = 70,
  color = "var(--emp)",
  dashed = false,
}: {
  points: (number | null)[];
  width?: number;
  height?: number;
  color?: string;
  dashed?: boolean;
}) {
  const vals = points.filter((v): v is number => v !== null && v !== undefined);
  if (vals.length < 2) {
    return (
      <svg className="spark mt14" width="100%" height={height} viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none">
        <line x1="0" y1={height / 2} x2={width} y2={height / 2} stroke="var(--line2)" strokeDasharray="4 4" />
      </svg>
    );
  }
  const min = Math.min(...vals);
  const max = Math.max(...vals);
  const span = max - min || 1;
  const step = width / (points.length - 1);
  const pts = points
    .map((v, i) => {
      if (v === null || v === undefined) return null;
      const x = i * step;
      const y = height - 6 - ((v - min) / span) * (height - 12);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .filter(Boolean)
    .join(" ");
  return (
    <svg className="spark mt14" width="100%" height={height} viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none">
      <polyline
        points={pts}
        fill="none"
        stroke={color}
        strokeWidth="2.5"
        strokeDasharray={dashed ? "5 5" : undefined}
      />
    </svg>
  );
}

export function Loading({ text = "加载中…" }: { text?: string }) {
  return <p className="sub">{text}</p>;
}

export function Err({ e }: { e: unknown }) {
  return <p className="err">{String(e)}</p>;
}

export function fmtDelta(d: number | null) {
  if (d === null) return null;
  return <span className={`delta ${d >= 0 ? "up" : "down"}`}>{d >= 0 ? "+" : ""}{d}</span>;
}
