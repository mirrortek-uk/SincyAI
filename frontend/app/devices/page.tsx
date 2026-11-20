"use client";

import { useEffect, useState } from "react";
import { api, type DeviceItem } from "@/lib/api";

// 设备页: 本租户注册的 Agent 设备及在线状态 (心跳 120s 内视为在线)。
export default function DevicesPage() {
  const [items, setItems] = useState<DeviceItem[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .listDevices()
      .then((r) => setItems(r.items))
      .catch((e) => setError(String(e)));
  }, []);

  return (
    <div>
      <h1>设备</h1>
      {error && <p style={{ color: "#e06c5a" }}>错误: {error}</p>}
      <div className="panel">
        <table cellPadding={8}>
          <thead>
            <tr>
              <th>状态</th>
              <th>名称</th>
              <th>平台</th>
              <th>系统 / Agent 版本</th>
              <th>最后心跳</th>
            </tr>
          </thead>
          <tbody>
            {items.map((d) => {
              const online =
                d.state === "active" &&
                d.last_seen &&
                Date.now() - new Date(d.last_seen).getTime() < 120_000;
              return (
                <tr key={d.id}>
                  <td>{online ? "🟢 online" : d.state}</td>
                  <td>{d.name ?? d.id.slice(0, 8)}</td>
                  <td>{d.platform}</td>
                  <td className="muted">
                    {d.os_version ?? "?"} / {d.agent_version ?? "?"}
                  </td>
                  <td className="muted">
                    {d.last_seen ? new Date(d.last_seen).toLocaleString() : "-"}
                  </td>
                </tr>
              );
            })}
            {items.length === 0 && (
              <tr>
                <td colSpan={5} className="muted">
                  暂无设备。Android Agent 安装后通过 challenge-response 注册接入。
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
