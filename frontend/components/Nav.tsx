"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { api, getToken, setToken, type Me } from "@/lib/api";

// 顶部导航: 未登录跳转 /login; 显示当前机构与角色。
export function Nav() {
  const pathname = usePathname();
  const router = useRouter();
  const [me, setMe] = useState<Me | null>(null);

  useEffect(() => {
    if (pathname === "/login") return;
    if (!getToken()) {
      router.push("/login");
      return;
    }
    api
      .me()
      .then(setMe)
      .catch(() => {
        setToken(null);
        router.push("/login");
      });
  }, [pathname, router]);

  if (pathname === "/login") return null;

  return (
    <nav className="topnav">
      <span className="brand">Sincy Console</span>
      <Link href="/">总览</Link>
      <Link href="/tasks">任务</Link>
      <Link href="/employees">数字员工</Link>
      <Link href="/intel">世界情报</Link>
      <Link href="/devices">设备</Link>
      <Link href="/ai">AI 生成</Link>
      <Link href="/kill-switches">急停开关</Link>
      <Link href="/metrics">指标</Link>
      <Link href="/architecture">架构</Link>
      <span style={{ flex: 1 }} />
      {me && (
        <span className="muted">
          {me.name} · {me.role}
        </span>
      )}
      <button
        style={{ background: "transparent", border: "1px solid var(--border)" }}
        onClick={() => {
          setToken(null);
          router.push("/login");
        }}
      >
        退出
      </button>
    </nav>
  );
}
