# 本地开发环境

```bash
docker run -d --name sincy-pg -e POSTGRES_PASSWORD=sincy -p 5432:5432 postgres:16
docker run -d --name sincy-nats -p 4222:4222 nats:2 -js
docker run -d --name sincy-redis -p 6379:6379 redis:7
```

首次运行迁移：`cargo run -p cloud-api`（启动时执行 migrations/，WP-03 起生效）。

## 前端 dev server 缓存污染（已知坑）

改动 `frontend/` 代码时如果 dev server 正在运行，`.next` 缓存会污染，表现为
`Cannot find module './319.js'` 500、或 React hydration error #418。
解法：杀掉 node 进程 → `rm -rf frontend/.next` → 重新 `npm run dev`。
