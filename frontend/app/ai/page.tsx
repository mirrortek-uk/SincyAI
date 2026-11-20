"use client";

import { useEffect, useState } from "react";
import { api, getToken, type AiGenerateResponse, type BrandItem } from "@/lib/api";

// AI 生成页 (FR-026): 走预算熔断保护的 LLM 生成; mock 结果会明确标注。
export default function AiPage() {
  const [prompt, setPrompt] = useState("");
  const [maxTokens, setMaxTokens] = useState(512);
  const [result, setResult] = useState<AiGenerateResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [brands, setBrands] = useState<BrandItem[]>([]);
  const [brandId, setBrandId] = useState("");
  const [saveAsPackage, setSaveAsPackage] = useState(true);

  useEffect(() => {
    if (!getToken()) return;
    api.listBrands().then((r) => {
      setBrands(r.items);
      if (r.items.length > 0) setBrandId(r.items[0].id);
    }).catch(() => {});
  }, []);

  async function run() {
    setError(null);
    setResult(null);
    try {
      const r = await api.aiGenerate({
        prompt,
        max_tokens: maxTokens,
        save_as_package:
          saveAsPackage && brandId
            ? { brand_id: brandId, platform: "x", action: "publish_post" }
            : undefined,
      });
      setResult(r);
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div>
      <h1>AI 生成</h1>
      <div className="panel">
        <label>提示词</label>
        <textarea
          style={{
            width: "100%",
            minHeight: 120,
            background: "#0b0d11",
            color: "var(--text)",
            border: "1px solid var(--border)",
            borderRadius: 6,
            font: "inherit",
            padding: 8,
          }}
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder="例如: 为品牌写一条 15 秒短视频口播文案…"
        />
        <label>max_tokens</label>
        <input
          type="number"
          value={maxTokens}
          onChange={(e) => setMaxTokens(Number(e.target.value))}
        />
        <label>Brand</label>
        <select value={brandId} onChange={(e) => setBrandId(e.target.value)}>
          {brands.map((b) => (
            <option key={b.id} value={b.id}>{b.name}</option>
          ))}
        </select>
        <label style={{ display: "flex", gap: 8, alignItems: "center", marginTop: 8 }}>
          <input
            type="checkbox"
            checked={saveAsPackage}
            onChange={(e) => setSaveAsPackage(e.target.checked)}
          />
          save as X package (publish_post, pending review, then publishable)
        </label>
        <div style={{ marginTop: 12 }}>
          <button onClick={run} disabled={!prompt || (saveAsPackage && !brandId)}>
            生成
          </button>
        </div>
      </div>
      {error && <p style={{ color: "#e06c5a" }}>错误: {error}</p>}
      {result && (
        <div className="panel">
          <h3>
            结果{" "}
            {result.mock && (
              <span style={{ color: "#e0a75a", fontSize: 13 }}>（mock 输出, 非真实模型）</span>
            )}
          </h3>
          <p style={{ whiteSpace: "pre-wrap" }}>{result.text}</p>
          <p className="muted">
            模型: {result.model} · tokens: {result.tokens_in}/{result.tokens_out} · 预估成本: $
            {result.estimated_cost_usd}
          </p>
          {result.package_id && (
            <p style={{ color: "#7fc97f" }}>
              saved as package <code>{result.package_id}</code> (pending review; approve it on the tasks page, then publish to X)
            </p>
          )}
        </div>
      )}
    </div>
  );
}
