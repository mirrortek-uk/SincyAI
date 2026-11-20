"use client";

import { useCallback, useEffect, useState } from "react";
import { api, type PublishTaskResponse, type TaskItem } from "@/lib/api";

// 任务页: 发布新任务 (FR-019 路由 + FR-020 配额 + 幂等) + 任务列表。
export default function TasksPage() {
  const [tasks, setTasks] = useState<TaskItem[]>([]);
  const [brandId, setBrandId] = useState("");
  const [packageId, setPackageId] = useState("");
  const [platform, setPlatform] = useState("youtube");
  const actions = platform === "x" ? ["publish_post", "read_metrics"] : platform === "youtube" ? ["publish_video", "publish_image", "read_metrics"] : ["publish_video"];
  const [action, setAction] = useState("publish_video");
  const [result, setResult] = useState<PublishTaskResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const r = await api.listTasks();
      setTasks(r.items);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  async function onPublish() {
    setError(null);
    setResult(null);
    try {
      const key = `web-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
      const res = await api.publishTask({
        brand_id: brandId,
        platform,
        action,
        package_id: packageId,
        package_version: 1,
        idempotency_key: key,
      });
      setResult(res);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div>
      <h1>发布任务</h1>
      <div className="panel">
        <label>Brand ID</label>
        <input value={brandId} onChange={(e) => setBrandId(e.target.value)} placeholder="uuid" size={40} />
        <label>Package ID（已审批的发布包）</label>
        <input value={packageId} onChange={(e) => setPackageId(e.target.value)} placeholder="uuid" size={40} />
        <label>平台 / 动作</label>
        <div className="row">
          <select
            value={platform}
            onChange={(e) => {
              setPlatform(e.target.value);
              setAction(e.target.value === "x" ? "publish_post" : "publish_video");
            }}
          >
            <option value="youtube">youtube</option>
            <option value="x">x (Twitter)</option>
          </select>
          <select value={action} onChange={(e) => setAction(e.target.value)}>
            {actions.map((a) => (
              <option key={a} value={a}>
                {a}
              </option>
            ))}
          </select>
        </div>
        <div style={{ marginTop: 12 }}>
          <button onClick={onPublish} disabled={!brandId || !packageId}>
            发布
          </button>
        </div>
      </div>
      {error && <p style={{ color: "#e06c5a" }}>错误: {error}</p>}
      {result && (
        <div className="panel">
          <h3>
            结果（路由: {result.route} / {result.status}）
          </h3>
          <pre>{JSON.stringify(result, null, 2)}</pre>
        </div>
      )}
      <div className="panel">
        <h3>最近任务</h3>
        <table cellPadding={8}>
          <thead>
            <tr>
              <th>状态</th>
              <th>路由</th>
              <th>平台/动作</th>
              <th>幂等键</th>
              <th>创建时间</th>
            </tr>
          </thead>
          <tbody>
            {tasks.map((t) => (
              <tr key={t.id}>
                <td>{t.state}</td>
                <td>{t.route ?? "-"}</td>
                <td className="muted">
                  {t.platform ?? "-"}/{t.action ?? "-"}
                </td>
                <td className="muted">{t.idempotency_key.slice(0, 18)}…</td>
                <td className="muted">{new Date(t.created_at).toLocaleString()}</td>
              </tr>
            ))}
            {tasks.length === 0 && (
              <tr>
                <td colSpan={5} className="muted">
                  暂无任务
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
