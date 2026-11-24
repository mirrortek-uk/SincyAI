import type { Metadata } from "next";

import "./globals.css";
import { Shell } from "@/components/Shell";

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
      <head>
        <script
          dangerouslySetInnerHTML={{
            __html:
              "try{var t=localStorage.getItem('sincy-theme');if(t==='dark')document.documentElement.setAttribute('data-theme','dark');}catch(e){}",
          }}
        />
      </head>
      <body>
        <Shell>{children}</Shell>
      </body>
    </html>
  );
}
