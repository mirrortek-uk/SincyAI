/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // 同源代理: 浏览器只访问 /api/v1 (Next rewrites 转发到后端),
  // 生产环境 console 与 cloud-api 同域部署时零 CORS 问题。
  // 后端地址: SINCY_API_INTERNAL (默认本地生产服务 18081, 见 /d/tmp/start-cloud-api-prod.sh)。
  async rewrites() {
    const backend = process.env.SINCY_API_INTERNAL ?? "http://127.0.0.1:18081";
    return [{ source: "/api/v1/:path*", destination: `${backend}/api/v1/:path*` }];
  },
};

export default nextConfig;
