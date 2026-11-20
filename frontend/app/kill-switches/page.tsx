"use client";

import { useEffect, useState } from "react";
import { api, type KillSwitch } from "@/lib/api";

// 急停开关 (FR-025)：作用域 global/agency/brand/client，1 分钟内生效。
export default function KillSwitchesPage() {
  const [agencyId, setAgencyId] = useState("");
  const [items, setItems] = useState<KillSwitch[]>([]);
  const [scope, setScope] = useState("agency");
  const [reason, setReason] = useState("");
  const [error, setError] = useState<string | null>(null);

  async function load() {
    try {
      const r = await api.listKillSwitches(agencyId || undefined);
      setItems(r.items);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agencyId]);

  async function create() {
    try {
      await api.createKillSwitch({
        agency_id: agencyId || undefined,
        scope,
        reason: reason || "manual",
      });
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
        <label>Agency ID（可选，过滤）</label>
        <input value={agencyId} onChange={(e) => setAgencyId(e.target.value)} placeholder="uuid" size={40} />
        <div className="row" style={{ marginTop: 8 }}>
          <select value={scope} onChange={(e) => setScope(e.target.value)}>
            <option value="global">global</option>
            <option value="agency">agency</option>
            <option value="brand">brand</option>
            <option value="client">client</option>
          </select>
          <input value={reason} onChange={(e) => setReason(e.target.value)} placeholder="原因" size={30} />
          <button onClick={create}>启用急停</button>
        </div>
      </div>
      {error && <p className="muted">错误: {error}</p>}
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
                <td className="muted">{k.created_at}</td>
                <td>
                  {k.active && (
                    <button className="danger" onClick={() => deactivate(k.id)}>
                      解除
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
