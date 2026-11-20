"use client";

import { useState } from "react";
import { api, type PublishTaskResponse } from "@/lib/api";

// 发布任务：FR-019 路由 + FR-020 配额 + 幂等键。结果含 route/status/fallback。
export default function TasksPage() {
  const [agencyId, setAgencyId] = useState("");
  const [platform, setPlatform] = useState("youtube");
  const [action, setAction] = useState("publish_video");
  const [result, setResult] = useState<PublishTaskResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [routeInfo, setRouteInfo] = useState<string | null>(null);

  async function onPublish() {
    setError(null);
    setResult(null);
    setRouteInfo(null);
    try {
      const key = `web-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
      const res = await api.publishTask({
        agency_id: agencyId,
        platform,
        action,
        idempotency_key: key,
        payload: { title: "demo", note: "from console" },
      });
      setResult(res);
      try {
        const route = await api.taskRoute(res.task_id);
        setRouteInfo(JSON.stringify(route, null, 2));
      } catch {
        // route audit row may not exist for assisted
      }
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div>
      <h1>发布任务</h1>
      <div className="panel">
        <label>Agency ID</label>
        <input value={agencyId} onChange={(e) => setAgencyId(e.target.value)} placeholder="uuid" size={40} />
        <label>平台</label>
        <input value={platform} onChange={(e) => setPlatform(e.target.value)} />
        <label>动作</label>
        <select value={action} onChange={(e) => setAction(e.target.value)}>
          <option value="publish_video">publish_video</option>
          <option value="publish_image">publish_image</option>
          <option value="read_metrics">read_metrics</option>
        </select>
        <div style={{ marginTop: 12 }}>
          <button onClick={onPublish} disabled={!agencyId}>
            发布
          </button>
        </div>
      </div>
      {error && <p className="muted">错误: {error}</p>}
      {result && (
        <div className="panel">
          <h3>结果（路由: {result.route} / {result.status}）</h3>
          <pre>{JSON.stringify(result, null, 2)}</pre>
        </div>
      )}
      {routeInfo && (
        <div className="panel">
          <h3>路由审计</h3>
          <pre>{routeInfo}</pre>
        </div>
      )}
    </div>
  );
}
