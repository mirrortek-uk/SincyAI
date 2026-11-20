"use client";

import { useEffect, useState } from "react";
import { api, type Health, type VersionInfo } from "@/lib/api";

export default function OverviewPage() {
  const [health, setHealth] = useState<Health | null>(null);
  const [version, setVersion] = useState<VersionInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([api.health(), api.version()])
      .then(([h, v]) => {
        setHealth(h);
        setVersion(v);
      })
      .catch((e) => setError(String(e)));
  }, []);

  return (
    <div>
      <h1>总览</h1>
      {error && <p className="muted">API 不可达: {error}（设置 NEXT_PUBLIC_API_BASE 指向 cloud-api）</p>}
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
