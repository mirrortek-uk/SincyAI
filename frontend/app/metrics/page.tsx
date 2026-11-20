"use client";

import { useEffect, useState } from "react";
import { api, getToken } from "@/lib/api";

// /metrics 的 Prometheus 文本直出，便于运维核对（WP-15）。
export default function MetricsPage() {
  const [text, setText] = useState("loading...");
  useEffect(() => {
    if (!getToken()) return;
    api.metrics().then(setText).catch((e) => setText(String(e)));
  }, []);
  return (
    <div>
      <h1>指标（/api/v1/metrics）</h1>
      <div className="panel">
        <pre>{text}</pre>
      </div>
    </div>
  );
}
