"use client";

import { useEffect, useState } from "react";
import { api, type KillSwitch } from "@/lib/api";

// 急停开关 (FR-025)：本租户 + 全局急停，1 分钟内生效，解除需审计动作。
export default function KillSwitchesPage() {
  const [items, setItems] = useState<KillSwitch[]>([]);
  const [scope, setScope] = useState("agency");
  const [reason, setReason] = useState("");
  const [error, setError] = useState<string | null>(null);

  async function load() {
    try {
      const r = await api.listKillSwitches();
      setItems(r.items);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    load();
  }, []);

  async function create() {
    try {
      await api.createKillSwitch({ scope, reason: reason || "manual" });
      setReason("");
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function deactivate(id: string) {
    try {
      await api.deactivateKillSwitch(id);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div>
      <h1>急停开关</h1>
      <div className="panel">
        <div className="row">
          <select value={scope} onChange={(e) => setScope(e.target.value)}>
            <option value="agency">agency（本机构）</option>
            <option value="brand">brand</option>
            <option value="client">client</option>
            <option value="global">global（全局,慎用）</option>
          </select>
          <input value={reason} onChange={(e) => setReason(e.target.value)} placeholder="原因" size={30} />
          <button onClick={create}>启用急停</button>
        </div>
      </div>
      {error && <p style={{ color: "#e06c5a" }}>错误: {error}</p>}
      <div className="panel">
        <table cellPadding={8}>
          <thead>
            <tr>
              <th>scope</th>
              <th>状态</th>
              <th>原因</th>
              <th>创建时间</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {items.map((k) => (
              <tr key={k.id}>
                <td>{k.scope}</td>
                <td>{k.active ? "ACTIVE" : "off"}</td>
                <td>{k.reason}</td>
                <td className="muted">{new Date(k.created_at).toLocaleString()}</td>
                <td>
                  {k.active && (
                    <button className="danger" onClick={() => deactivate(k.id)}>
                      解除
                    </button>
                  )}
                </td>
              </tr>
            ))}
            {items.length === 0 && (
              <tr>
                <td colSpan={5} className="muted">
                  当前无生效急停
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
