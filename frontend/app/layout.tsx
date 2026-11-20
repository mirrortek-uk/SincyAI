import type { Metadata } from "next";
import Link from "next/link";
import "./globals.css";
import { Nav } from "@/components/Nav";

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
        <Nav />
        <main className="content">{children}</main>
      </body>
    </html>
  );
}
