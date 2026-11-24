"use client";

// 🎙️ 语气档案 (Voice Profile): 员工基线 ⊕ 账号覆盖 = 有效语气
//   1) 选员工 → 看/编辑基线 voice_profile (结构化字段 + JSON)
//   2) 蒸馏: 粘贴该账号的公开帖子采样 → LLM 压成结构化档案 → 一键保存为基线或账号覆盖
//   3) 账号覆盖列表 + 编辑; 有效语气 = merge_opt(基线, 覆盖), 附编译后的 prompt 指令块预览

import { useEffect, useState } from "react";

import {
  api,
  type EmployeeItem,
  type VoiceAccount,
  type VoiceProfileJson,
} from "@/lib/api";
import { Badge, Err, Loading } from "@/components/ui";

const FIELDS: { key: string; label: string; placeholder?: string }[] = [
  { key: "register", label: "基调 register", placeholder: "witty-authoritative / 犀利 / 温暖…" },
  { key: "formality", label: "正式度 0-1", placeholder: "0.3" },
  { key: "person", label: "人称", placeholder: "first_person / brand_we" },
  { key: "emoji", label: "emoji", placeholder: "none / sparing / liberal" },
  { key: "rhythm", label: "节奏", placeholder: "punchy / long_form / mixed" },
  { key: "sentence_len", label: "句长", placeholder: "short / medium / long" },
  { key: "humor", label: "幽默", placeholder: "dry / playful / none" },
  { key: "slang", label: "俚语", placeholder: "light / none" },
  { key: "capitalization", label: "大小写习惯", placeholder: "lowercase_casual / standard" },
  { key: "pov_values", label: "立场/价值观", placeholder: "pragmatic builder, anti-hype" },
  { key: "signature_phrases", label: "口头禅 (逗号分隔)", placeholder: "ship it, no cap" },
  { key: "avoid_words", label: "禁用词 (逗号分隔, 硬约束)", placeholder: "leverage, synergy" },
  { key: "banned_moves", label: "禁写手法 (逗号分隔, 硬约束)", placeholder: "engagement_bait, hard_sell" },
  { key: "structure_habits", label: "结构套路 (逗号分隔)", placeholder: "hook_first, one_idea_per_post" },
];

function toList(v: unknown): string {
  return Array.isArray(v) ? v.join(", ") : typeof v === "string" ? v : "";
}
function fromList(s: string): string[] {
  return s.split(",").map((x) => x.trim()).filter(Boolean);
}

export function VoiceView() {
  const [employees, setEmployees] = useState<EmployeeItem[]>([]);
  const [empId, setEmpId] = useState("");
  const [voice, setVoice] = useState("");
  const [profile, setProfile] = useState<VoiceProfileJson>({});
  const [rawJson, setRawJson] = useState("");
  const [accounts, setAccounts] = useState<VoiceAccount[]>([]);
  const [effective, setEffective] = useState<{ effective: VoiceProfileJson; prompt_block: string } | null>(null);
  const [samples, setSamples] = useState("");
  const [handles, setHandles] = useState("");
  const [distilled, setDistilled] = useState<VoiceProfileJson | null>(null);
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [ok, setOk] = useState("");

  useEffect(() => {
    api.listEmployees().then((r) => {
      setEmployees(r.items);
      if (r.items[0]) setEmpId(r.items[0].id);
    }).catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    if (!empId) return;
    setError(""); setOk("");
    api.getEmployeeVoice(empId).then((r) => {
      setVoice(typeof r.voice === "string" ? r.voice : "");
      setProfile(r.voice_profile || {});
      setRawJson(JSON.stringify(r.voice_profile || {}, null, 2));
      const emp = employees.find((e) => e.id === empId);
      return api.getVoiceEffective(empId, emp?.channel || "x").then((eff) => setEffective(eff));
    }).catch((e) => setError(String(e)));
    api.listVoiceAccounts(empId).then((r) => setAccounts(r.items)).catch(() => {});
  }, [empId]); // eslint-disable-line react-hooks/exhaustive-deps

  const saveBase = async () => {
    setBusy("save"); setError(""); setOk("");
    try {
      let vp: VoiceProfileJson = profile;
      if (rawJson.trim()) vp = JSON.parse(rawJson); // JSON 编辑器优先
      await api.putEmployeeVoice(empId, { voice: voice || undefined, voice_profile: vp });
      setProfile(vp);
      setOk("基线已保存 (新 identity 版本)");
    } catch (e) { setError(String(e)); } finally { setBusy(""); }
  };

  const runDistill = async () => {
    setBusy("distill"); setError(""); setOk(""); setDistilled(null);
    try {
      const arr = samples.split("\n---\n").map((s) => s.trim()).filter(Boolean);
      const hs = fromList(handles);
      const r = await api.distillVoice(arr, hs);
      setDistilled(r.profile);
      setOk("蒸馏完成, 可检查后保存");
    } catch (e) { setError(String(e)); } finally { setBusy(""); }
  };

  const saveDistilledAsBase = async () => {
    if (!distilled) return;
    setBusy("save"); setError("");
    try {
      await api.putEmployeeVoice(empId, { voice_profile: distilled });
      setProfile(distilled); setRawJson(JSON.stringify(distilled, null, 2));
      setOk("已保存为员工基线");
    } catch (e) { setError(String(e)); } finally { setBusy(""); }
  };

  const saveDistilledAsAccount = async (acc: VoiceAccount) => {
    if (!distilled) return;
    setBusy("acc" + acc.id); setError("");
    try {
      await api.putVoiceAccount({ employee_id: empId, platform: acc.platform, account_id: acc.account_id, voice_profile: distilled });
      setOk(`已保存为账号覆盖 ${acc.platform}/${acc.account_id}`);
      const r = await api.listVoiceAccounts(empId); setAccounts(r.items);
    } catch (e) { setError(String(e)); } finally { setBusy(""); }
  };

  const saveAccountJson = async (acc: VoiceAccount, jsonText: string) => {
    setBusy("acc" + acc.id); setError(""); setOk("");
    try {
      const vp = JSON.parse(jsonText);
      await api.putVoiceAccount({ employee_id: empId, platform: acc.platform, account_id: acc.account_id, voice_profile: vp });
      const r = await api.listVoiceAccounts(empId); setAccounts(r.items);
      setOk(`已保存 ${acc.platform}/${acc.account_id} 覆盖`);
    } catch (e) { setError(String(e)); } finally { setBusy(""); }
  };

  const emp = employees.find((e) => e.id === empId);

  return (
    <div>
      <h2>🎙️ 语气档案 <span className="sub">Voice Profile — 员工基线 ⊕ 账号覆盖, 注入工序2/3 与质量门硬拦</span></h2>
      {error && <Err e={error} />}
      {ok && <p className="ok">{ok}</p>}

      <div className="card">
        <div className="row" style={{ gap: 8, alignItems: "center" }}>
          <b>员工</b>
          <select value={empId} onChange={(e) => setEmpId(e.target.value)}>
            {employees.map((e) => <option key={e.id} value={e.id}>{e.name} ({e.channel})</option>)}
          </select>
          {emp && <Badge kind="run">{emp.status}</Badge>}
          <span className="sub">基线存 identity.voice_profile · 覆盖存 employee_accounts.voice_profile (0047)</span>
        </div>
      </div>

      <div className="grid2">
        <div className="card">
          <h3>① 员工基线 (base)</h3>
          <label className="sub">Voice 单字段 (兼容旧 prompt)</label>
          <input value={voice} onChange={(e) => setVoice(e.target.value)} placeholder="neutral / casual, human" />
          {FIELDS.map((f) => (
            <div key={f.key}>
              <label className="sub">{f.label}</label>
              <input
                value={toList(profile[f.key])}
                placeholder={f.placeholder}
                onChange={(e) => setProfile({ ...profile, [f.key]: f.key.includes("words") || f.key.includes("moves") || f.key.includes("phrases") || f.key.includes("habits") ? fromList(e.target.value) : e.target.value })}
              />
            </div>
          ))}
          <label className="sub">完整 JSON (优先以此为准, 可直接粘贴蒸馏结果)</label>
          <textarea rows={8} value={rawJson} onChange={(e) => setRawJson(e.target.value)} />
          <button disabled={busy === "save"} onClick={saveBase}>{busy === "save" ? "保存中…" : "保存基线"}</button>
        </div>

        <div className="card">
          <h3>② 从真实帖子蒸馏 (distill)</h3>
          <p className="sub">粘贴目标账号的公开帖子采样 (每条用 <code>---</code> 分隔, 建议 5-50 条; 仅用于风格分析, 不入长期记忆; 产物仍过 originality 门)</p>
          <textarea rows={8} value={samples} onChange={(e) => setSamples(e.target.value)} placeholder={"帖子 1\n---\n帖子 2\n---\n帖子 3"} />
          <label className="sub">来源 handle (逗号分隔, 记入 source_accounts)</label>
          <input value={handles} onChange={(e) => setHandles(e.target.value)} placeholder="@someone" />
          <button disabled={busy === "distill"} onClick={runDistill}>{busy === "distill" ? "蒸馏中…" : "蒸馏成语气档案"}</button>
          {distilled && (
            <div style={{ marginTop: 8 }}>
              <label className="sub">蒸馏结果 (可直接保存)</label>
              <textarea rows={8} readOnly value={JSON.stringify(distilled, null, 2)} />
              <div className="row" style={{ gap: 8 }}>
                <button disabled={busy === "save"} onClick={saveDistilledAsBase}>保存为员工基线</button>
                {accounts[0] && <button disabled={busy.startsWith("acc")} onClick={() => saveDistilledAsAccount(accounts[0])}>保存为账号覆盖 ({accounts[0].platform}/{accounts[0].account_id})</button>}
              </div>
            </div>
          )}
        </div>
      </div>

      <div className="card">
        <h3>③ 账号覆盖 (override) — {accounts.length} 个账号</h3>
        {accounts.length === 0 && <p className="sub">该员工还没有登记账号 (employee_accounts)。可在 KPI 页或设备绑定处登记。</p>}
        {accounts.map((a) => (
          <details key={a.id} style={{ marginBottom: 8 }}>
            <summary><b>{a.platform}</b> / {a.account_id} {Object.keys(a.voice_profile || {}).length > 0 ? <Badge kind="ok">已设覆盖</Badge> : <Badge kind="pend">基线生效</Badge>}</summary>
            <AccountEditor acc={a} busy={busy === "acc" + a.id} onSave={(t) => saveAccountJson(a, t)} />
          </details>
        ))}
      </div>

      <div className="card">
        <h3>④ 有效语气 (effective = 基线 ⊕ 覆盖) 与 prompt 指令块预览</h3>
        {!effective && <Loading />}
        {effective && (
          <>
            <pre style={{ maxHeight: 260, overflow: "auto" }}>{effective.prompt_block || "(空档案 — 生成时退回 Voice 单字段)"}</pre>
            <details><summary className="sub">effective JSON</summary>
              <pre style={{ maxHeight: 200, overflow: "auto" }}>{JSON.stringify(effective.effective, null, 2)}</pre>
            </details>
          </>
        )}
      </div>
    </div>
  );
}

function AccountEditor({ acc, busy, onSave }: { acc: VoiceAccount; busy: boolean; onSave: (t: string) => void }) {
  const [text, setText] = useState(JSON.stringify(acc.voice_profile || {}, null, 2));
  return (
    <div style={{ marginTop: 6 }}>
      <textarea rows={6} value={text} onChange={(e) => setText(e.target.value)} />
      <button disabled={busy} onClick={() => onSave(text)}>{busy ? "保存中…" : "保存该账号覆盖"}</button>
    </div>
  );
}
