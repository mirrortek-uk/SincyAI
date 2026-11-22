"use client";

import { useCallback, useEffect, useState } from "react";
import { api, getToken, type BrandItem, type PackageItem, type PublishTaskResponse, type TaskItem } from "@/lib/api";

// 任务页: 选品牌 + 选内容包 → 发布 (FR-019 路由 + FR-020 配额 + 幂等) + 任务列表。
// 内容包来自 "AI 生成页存包" 或手工创建 (GET /packages)。
export default function TasksPage() {
  const [tasks, setTasks] = useState<TaskItem[]>([]);
  const [brands, setBrands] = useState<BrandItem[]>([]);
  const [packages, setPackages] = useState<PackageItem[]>([]);
  const [brandId, setBrandId] = useState("");
  const [packageId, setPackageId] = useState("");
  const [platform, setPlatform] = useState("x");
  const actions = platform === "x" ? ["publish_post", "reply_comment", "read_metrics"] : platform === "youtube" ? ["publish_video", "publish_image", "read_metrics"] : ["publish_video"];
  const [action, setAction] = useState("publish_post");
  const [result, setResult] = useState<PublishTaskResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!getToken()) return; // 未登录不发请求
    try {
      const r = await api.listTasks();
      setTasks(r.items);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const loadMeta = useCallback(async () => {
    if (!getToken()) return;
    try {
      const b = await api.listBrands();
      setBrands(b.items);
      if (!brandId && b.items.length > 0) setBrandId(b.items[0].id);
    } catch (e) {
      setError(String(e));
    }
  }, [brandId]);

  const loadPackages = useCallback(async () => {
    if (!getToken()) return;
    try {
      const r = await api.listPackages(platform, action);
      setPackages(r.items);
      setPackageId((cur) => (r.items.some((p) => p.id === cur) ? cur : r.items[0]?.id ?? ""));
    } catch (e) {
      setError(String(e));
    }
  }, [platform, action]);

  useEffect(() => {
    load();
    loadMeta();
  }, [load, loadMeta]);

  useEffect(() => {
    loadPackages();
  }, [loadPackages]);

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
        <label>品牌</label>
        <select value={brandId} onChange={(e) => setBrandId(e.target.value)}>
          {brands.length === 0 && <option value="">（无品牌，先建品牌）</option>}
          {brands.map((b) => (
            <option key={b.id} value={b.id}>{b.name}</option>
          ))}
        </select>
        <label>内容包（X 内容从这里来：AI 生成页保存 / 手工创建）</label>
        <select value={packageId} onChange={(e) => setPackageId(e.target.value)}>
          {packages.length === 0 && <option value="">（暂无 {platform}/{action} 内容包）</option>}
          {packages.map((p) => (
            <option key={p.id} value={p.id}>
              [{p.approval_status}] {p.text_preview.slice(0, 40)}…
            </option>
          ))}
        </select>
        {(() => {
          const sel = packages.find((p) => p.id === packageId);
          if (!sel) return null;
          const prov = (sel.constraints?.provenance ?? {}) as Record<string, unknown>;
          return (
            <div className="panel" style={{ marginTop: 8 }}>
              <strong>已选内容包全文（审阅用）</strong>
              <p className="muted" style={{ margin: "4px 0" }}>
                id {sel.id.slice(0, 8)} · {sel.platform}/{sel.action} · 状态 {sel.approval_status}
                {prov.topic ? ` · topic=${String(prov.topic)}` : ""}
                {prov.employee_id ? ` · 数字员工 ${String(prov.employee_id).slice(0, 8)} 生成` : ""}
                {" "}· {sel.text_content.length} 字
              </p>
              <pre style={{
                whiteSpace: "pre-wrap",
                wordBreak: "break-word",
                background: "var(--bg)",
                border: "1px solid var(--border)",
                borderRadius: 6,
                padding: 10,
                margin: 0,
                maxHeight: 260,
                overflowY: "auto",
                fontSize: 13,
              }}>{sel.text_content}</pre>
            </div>
          );
        })()}
        {packages.find((p) => p.id === packageId)?.approval_status === "pending" && (
          <p className="muted" style={{ color: "#e0a75a" }}>
            该包待审核（AI 生成），发布前需先在下方"待审核包"里点通过
          </p>
        )}
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
        <h3>待审核内容包（AI 生成）</h3>
        <table cellPadding={8}>
          <tbody>
            {packages.filter((p) => p.approval_status === "pending").length === 0 && (
              <tr><td className="muted">无</td></tr>
            )}
            {packages.filter((p) => p.approval_status === "pending").map((p) => (
              <tr key={p.id}>
                <td className="muted">{p.platform}/{p.action}</td>
                <td style={{ maxWidth: 480 }}>
                  <pre style={{ whiteSpace: "pre-wrap", wordBreak: "break-word", margin: 0, fontSize: 12, maxHeight: 120, overflowY: "auto" }}>
                    {p.text_content}
                  </pre>
                </td>
                <td>
                  <button onClick={async () => { await api.approvePackage(p.id); await loadPackages(); }}>
                    审核通过
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
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
              <th>操作</th>
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
                <td>
                  {(t.state === "WAITING_USER" || t.state === "FAILED") && (
                    <button
                      onClick={() =>
                        api
                          .retryTask(t.id)
                          .then(() => load())
                          .catch((e) => setError(String(e)))
                      }
                    >
                      {t.state === "WAITING_USER" ? "已人工处理，重试" : "重试"}
                    </button>
                  )}
                </td>
              </tr>
            ))}
            {tasks.length === 0 && (
              <tr>
                <td colSpan={6} className="muted">
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
