"use client";

import { useEffect, useState } from "react";
import { api, getToken, type Health, type Me, type VersionInfo } from "@/lib/api";

export default function OverviewPage() {
  const [health, setHealth] = useState<Health | null>(null);
  const [version, setVersion] = useState<VersionInfo | null>(null);
  const [me, setMe] = useState<Me | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!getToken()) return; // 未登录: Nav 负责跳 /login, 这里不发请求避免 401 噪音
    Promise.all([api.health(), api.version(), api.me()])
      .then(([h, v, m]) => {
        setHealth(h);
        setVersion(v);
        setMe(m);
      })
      .catch((e) => setError(String(e)));
  }, []);

  return (
    <div>
      <h1>总览</h1>
      {me && (
        <div className="panel">
          <h3>{me.name}</h3>
          <p className="muted">
            机构 ID: {me.id} · 套餐: {me.plan} · 你的角色: {me.role}
          </p>
        </div>
      )}
      {error && <p className="muted">API 不可达: {error}</p>}
      <div className="row">
        <div className="panel">
          <h3>健康检查</h3>
          <pre>{health ? JSON.stringify(health, null, 2) : "loading..."}</pre>
        </div>
        <div className="panel">
          <h3>版本</h3>
          <pre>{version ? JSON.stringify(version, null, 2) : "loading..."}</pre>
        </div>
      </div>
    </div>
  );
}
