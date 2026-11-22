"use client";

import { useEffect, useState } from "react";
import { api, getToken, type DeviceItem } from "@/lib/api";

type SetupApp = {
  app_key: string;
  display_name: string;
  target_package: string;
  download_url?: string;
  sha256?: string;
};

// 设备页: 本租户注册的 Agent 设备及在线状态 (心跳 120s 内视为在线)。
export default function DevicesPage() {
  const [items, setItems] = useState<DeviceItem[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [pairingCode, setPairingCode] = useState<{
    code: string;
    expires_at: string;
  } | null>(null);
  const [codeError, setCodeError] = useState<string | null>(null);
  const [setupMsg, setSetupMsg] = useState<string | null>(null);
  const [setupApps, setSetupApps] = useState<SetupApp[]>([]);
  const [goal, setGoal] = useState("");
  const [editApp, setEditApp] = useState<Record<string, { url: string; sha: string }>>({});

  useEffect(() => {
    if (!getToken()) return;
    api.listSetupApps().then((r) => setSetupApps(r.items)).catch(() => {});
  }, []);

  const runSetupInstall = (app?: string) => {
    setSetupMsg(null);
    api
      .runSetup("setup_install", app)
      .then((r) => setSetupMsg(`已派发引导安装任务 ${r.task_id.slice(0, 8)} (${r.state}) — 看手机通知栏/任务页`))
      .catch((e) => setSetupMsg(`派发失败: ${e}`));
  };

  const runAgentic = () => {
    const g = goal.trim();
    if (!g) {
      setSetupMsg("请先输入目标 (goal)，例如: 打开设置里的电池页面并截图每一步操作");
      return;
    }
    setSetupMsg(null);
    api
      .runSetup("agentic", undefined, g)
      .then((r) =>
        setSetupMsg(
          `已派发 agentic 任务 ${r.task_id.slice(0, 8)} (${r.state}, adapter=${r.adapter ?? "?"}) — 手机 agent 会逐屏发给云端 LLM 决策执行; 任务页可看每步证据`,
        ),
      )
      .catch((e) => setSetupMsg(`派发失败: ${e}`));
  };

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
        <p>
          {setupApps.map((a) => (
            <span key={a.app_key} style={{ marginRight: 16 }}>
              <button onClick={() => runSetupInstall(a.app_key)}>引导安装 {a.display_name}</button>{" "}
              <button
                onClick={() =>
                  setEditApp((cur) => ({
                    ...cur,
                    [a.app_key]: cur[a.app_key] ?? { url: a.download_url ?? "", sha: a.sha256 ?? "" },
                  }))
                }
              >
                {editApp[a.app_key] ? "收起" : "配置下载地址"}
              </button>
              {editApp[a.app_key] && (
                <span style={{ display: "inline-block", marginLeft: 8 }}>
                  <input
                    placeholder="https:// 白名单 APK 直链"
                    style={{ width: 300, marginRight: 4 }}
                    value={editApp[a.app_key].url}
                    onChange={(e) =>
                      setEditApp((cur) => ({
                        ...cur,
                        [a.app_key]: { ...cur[a.app_key], url: e.target.value.trim() },
                      }))
                    }
                  />
                  <input
                    placeholder="APK SHA-256 (64 hex)"
                    style={{ width: 260, marginRight: 4 }}
                    value={editApp[a.app_key].sha}
                    onChange={(e) =>
                      setEditApp((cur) => ({
                        ...cur,
                        [a.app_key]: { ...cur[a.app_key], sha: e.target.value.trim() },
                      }))
                    }
                  />
                  <button
                    onClick={() => {
                      const v = editApp[a.app_key];
                      api
                        .updateSetupApp(a.app_key, v.url, v.sha)
                        .then(() => setSetupMsg(`已更新 ${a.display_name} 的下载地址/SHA-256`))
                        .catch((e) => setSetupMsg(`更新失败: ${e}`));
                    }}
                  >
                    保存
                  </button>
                </span>
              )}
            </span>
          ))}
          {setupApps.length === 0 && (
            <button onClick={() => runSetupInstall()}>引导安装（通用）</button>
          )}{" "}
          <span className="muted">
            手机 agent 会下载安装包、校验 SHA-256、调起系统安装器（需你手点「安装」），完成后到登录页等你输入账号密码
          </span>
        </p>
        <p>
          <input
            value={goal}
            onChange={(e) => setGoal(e.target.value)}
            placeholder="agentic 目标 (自然语言), 如: 帮我在设置里打开开发者选项"
            style={{ width: 420, marginRight: 8 }}
          />
          <button onClick={runAgentic}>派发 agentic 任务</button>{" "}
          <span className="muted">
            「真人模式」: 手机每步截屏发给云端 LLM 决定下一步点击/滑动/输入 (只允许安全动作, 安装/发布仍走签名 adapter; 遇到验证页/锁屏自动转人工 — 解锁手机后在任务页点「已人工处理，重试」继续)
          </span>
        </p>
        {setupMsg && <p className="muted">{setupMsg}</p>}
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
