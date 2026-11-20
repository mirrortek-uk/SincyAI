"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { api, setToken } from "@/lib/api";

// 登录 / 注册页: 正式产品入口。注册 = 创建机构 + owner 账号。
export default function LoginPage() {
  const router = useRouter();
  const [mode, setMode] = useState<"login" | "register">("login");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit() {
    setBusy(true);
    setError(null);
    try {
      if (mode === "login") {
        const r = await api.login(email, password);
        setToken(r.token);
      } else {
        const r = await api.registerAgency(name, email, password);
        if (r.token) setToken(r.token);
      }
      router.push("/");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={{ maxWidth: 380, margin: "80px auto" }}>
      <form
        className="panel"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <h2 style={{ marginTop: 0 }}>{mode === "login" ? "登录 Sincy Console" : "注册机构账号"}</h2>
        {mode === "register" && (
          <>
            <label>机构名称</label>
            <input value={name} onChange={(e) => setName(e.target.value)} placeholder="我的内容机构" />
          </>
        )}
        <label>邮箱</label>
        <input
          type="email"
          autoComplete="username"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="owner@agency.dev"
        />
        <label>密码（至少 8 位）</label>
        <input
          type="password"
          autoComplete={mode === "login" ? "current-password" : "new-password"}
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        {error && <p className="muted" style={{ color: "#e06c5a" }}>错误: {error}</p>}
        <div style={{ marginTop: 14, display: "flex", gap: 8 }}>
          <button type="submit" disabled={busy || !email || !password || (mode === "register" && !name)}>
            {mode === "login" ? "登录" : "创建机构并登录"}
          </button>
          <button
            type="button"
            style={{ background: "transparent", border: "1px solid var(--border)" }}
            onClick={() => setMode(mode === "login" ? "register" : "login")}
          >
            {mode === "login" ? "没有账号? 注册" : "已有账号? 登录"}
          </button>
        </div>
      </form>
    </div>
  );
}
