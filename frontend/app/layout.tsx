import type { Metadata } from "next";
import Link from "next/link";
import "./globals.css";

export const metadata: Metadata = {
  title: "Sincy Console",
  description: "Sincy Agency Console — 多租户 AI 内容运营平台",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="zh-CN">
      <body>
        <nav className="topnav">
          <span className="brand">Sincy Console</span>
          <Link href="/">总览</Link>
          <Link href="/tasks">发布任务</Link>
          <Link href="/kill-switches">急停开关</Link>
          <Link href="/metrics">指标</Link>
        </nav>
        <main className="content">{children}</main>
      </body>
    </html>
  );
}
