"use client";

import { useCallback, useEffect, useState } from "react";
import { api, getToken, type ReviewRule } from "@/lib/api";

// 设置页: 平台审核门 (每个平台的加权字数上限 + 超限行为)。
// 默认值来自 platform_review_rules 表迁移种子, 这里可修改并保存。
export default function SettingsPage() {
  const [rules, setRules] = useState<ReviewRule[]>([]);
  const [drafts, setDrafts] = useState<Record<string, { weighted_limit: string; on_over_limit: string; note: string }>>({});
  const [msg, setMsg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    if (!getToken()) return;
    try {
      const r = await api.listReviewRules();
      setRules(r.items);
      const d: Record<string, { weighted_limit: string; on_over_limit: string; note: string }> = {};
      for (const it of r.items) d[it.platform] = { weighted_limit: String(it.weighted_limit), on_over_limit: it.on_over_limit, note: it.note };
      setDrafts(d);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  async function save(platform: string) {
    setBusy(true);
    setError(null);
    setMsg(null);
    try {
      const d = drafts[platform];
      const limit = parseInt(d.weighted_limit, 10);
      if (!Number.isFinite(limit) || limit <= 0) throw new Error("字数上限必须是正整数");
      await api.upsertReviewRule(platform, { weighted_limit: limit, on_over_limit: d.on_over_limit, note: d.note });
      setMsg(`已保存 ${platform} 的审核门`);
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <main>
      <h1>设置</h1>
      <p className="muted">
        平台审核门: 每个平台发布内容前的字数上限(加权, 中文/CJK 字符按 2 计)与超限行为。
        超限行为: <b>reject</b> = 打回不发布; <b>thread</b> = 自动拆成多条串行帖发布(如 X)。
        修改后对之后生成的内容生效。
      </p>
      {error && <p style={{ color: "var(--danger)" }}>{error}</p>}
      {msg && <p style={{ color: "var(--accent)" }}>{msg}</p>}

      <section className="panel">
        <h2>平台审核门</h2>
        <table>
          <thead>
            <tr><th>平台</th><th>加权字数上限</th><th>超限行为</th><th>备注</th><th>更新时间</th><th>操作</th></tr>
          </thead>
          <tbody>
            {rules.map((r) => {
              const d = drafts[r.platform] ?? { weighted_limit: String(r.weighted_limit), on_over_limit: r.on_over_limit, note: r.note };
              return (
                <tr key={r.platform}>
                  <td><b>{r.platform}</b></td>
                  <td>
                    <input
                      type="number"
                      min={1}
                      style={{ width: 90 }}
                      value={d.weighted_limit}
                      onChange={(e) => setDrafts((cur) => ({ ...cur, [r.platform]: { ...d, weighted_limit: e.target.value } }))}
                    />
                  </td>
                  <td>
                    <select
                      value={d.on_over_limit}
                      onChange={(e) => setDrafts((cur) => ({ ...cur, [r.platform]: { ...d, on_over_limit: e.target.value } }))}
                    >
                      <option value="reject">reject — 超限打回</option>
                      <option value="thread">thread — 拆成串行帖发布</option>
                    </select>
                  </td>
                  <td>
                    <input
                      style={{ width: 220 }}
                      value={d.note}
                      placeholder="备注"
                      onChange={(e) => setDrafts((cur) => ({ ...cur, [r.platform]: { ...d, note: e.target.value } }))}
                    />
                  </td>
                  <td className="muted">{r.updated_at.slice(0, 19).replace("T", " ")}</td>
                  <td>
                    <button disabled={busy} onClick={() => save(r.platform)}>保存</button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
        <p className="muted" style={{ marginBottom: 0 }}>
          新增平台规则: 直接对未列出的平台名调用 PUT /api/v1/settings/platform-review-rules/{"{platform}"},
          未配置的平台自动使用 <b>default</b> 行的规则。
        </p>
      </section>
    </main>
  );
}
