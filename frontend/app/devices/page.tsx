"use client";

import { useEffect, useState } from "react";
import { api, getToken, type DeviceItem } from "@/lib/api";

// 设备页: 本租户注册的 Agent 设备及在线状态 (心跳 120s 内视为在线)。
export default function DevicesPage() {
  const [items, setItems] = useState<DeviceItem[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [pairingCode, setPairingCode] = useState<{
    code: string;
    expires_at: string;
  } | null>(null);
  const [codeError, setCodeError] = useState<string | null>(null);

  useEffect(() => {
    if (!getToken()) return;
    api
      .listDevices()
      .then((r) => setItems(r.items))
      .catch((e) => setError(String(e)));
  }, []);

  const genCode = () => {
    setCodeError(null);
    api
      .createPairingCode(10)
      .then(setPairingCode)
      .catch((e) => setCodeError(String(e)));
  };

  return (
    <div>
      <h1>设备</h1>
      {error && <p style={{ color: "#e06c5a" }}>错误: {error}</p>}
      <div className="panel">
        <p>
          <button onClick={genCode}>生成配对码</button>{" "}
          <span className="muted">
            在手机 Agent 里只输入这个 4 位数字码即可配对，无需账号密码 (配对成功后设备会话默认有效 60 天)
          </span>
        </p>
        {codeError && <p style={{ color: "#e06c5a" }}>生成失败: {codeError}</p>}
        {pairingCode && (
          <p
            style={{
              fontSize: 28,
              letterSpacing: 6,
              fontFamily: "monospace",
              background: "#eef4ff",
              display: "inline-block",
              padding: "8px 16px",
              borderRadius: 8,
            }}
          >
            {pairingCode.code}{" "}
            <span className="muted" style={{ fontSize: 12, letterSpacing: 0 }}>
              (有效期至 {new Date(pairingCode.expires_at).toLocaleTimeString()})
            </span>
          </p>
        )}
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
                  暂无设备。Android Agent 安装后输入配对码即可接入。
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
