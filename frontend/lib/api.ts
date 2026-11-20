// Typed client for the Sincy Cloud API (/api/v1). Base URL via
// NEXT_PUBLIC_API_BASE (defaults to the local dev server). The console JWT
// is stored in localStorage and attached as Authorization: Bearer.

// 默认走同源 /api/v1 (next.config.mjs rewrites 到后端); 需要直连后端时
// 设置 NEXT_PUBLIC_API_BASE=http://127.0.0.1:18080/api/v1 (需后端 CORS 放行).
const BASE = process.env.NEXT_PUBLIC_API_BASE ?? "/api/v1";

const TOKEN_KEY = "sincy.console.token";

export function getToken(): string | null {
  if (typeof window === "undefined") return null;
  return window.localStorage.getItem(TOKEN_KEY);
}

export function setToken(token: string | null) {
  if (typeof window === "undefined") return;
  if (token) window.localStorage.setItem(TOKEN_KEY, token);
  else window.localStorage.removeItem(TOKEN_KEY);
}

export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
  ) {
    super(message);
  }
}

async function req<T>(path: string, init?: RequestInit): Promise<T> {
  const token = getToken();
  const res = await fetch(`${BASE}${path}`, {
    ...init,
    headers: {
      "content-type": "application/json",
      ...(token ? { authorization: `Bearer ${token}` } : {}),
      ...init?.headers,
    },
  });
  if (!res.ok) {
    let code = "http_error";
    let message = `${res.status}`;
    try {
      const body = await res.json();
      if (body?.error) {
        code = body.error.code ?? code;
        message = body.error.message ?? message;
      }
    } catch {
      // non-JSON error body
    }
    if (res.status === 401) {
      setToken(null); // 令牌失效, 回登录页
    }
    throw new ApiError(res.status, code, message);
  }
  return res.json() as Promise<T>;
}

export interface Health {
  status: string;
  ts: string;
}

export interface VersionInfo {
  service: string;
  version: string;
  protocol_schema: number;
}

export interface Me {
  id: string;
  name: string;
  plan: string;
  role: string;
  member_id: string;
}

export interface PublishTaskResponse {
  task_id: string;
  route: string;
  status: string;
  external_content_id: string | null;
  receipt: unknown;
  fallback: string | null;
}

export interface TaskItem {
  id: string;
  state: string;
  idempotency_key: string;
  route: string | null;
  platform: string | null;
  action: string | null;
  package_id: string | null;
  created_at: string;
}

export interface DeviceItem {
  id: string;
  name: string | null;
  platform: string;
  os_version: string | null;
  agent_version: string | null;
  state: string;
  last_seen: string | null;
  created_at: string;
}

export interface KillSwitch {
  id: string;
  agency_id: string | null;
  scope: string;
  target_id: string | null;
  reason: string;
  active: boolean;
  created_at: string;
}

export interface AiGenerateResponse {
  text: string;
  model: string;
  tokens_in: number;
  tokens_out: number;
  mock: boolean;
  estimated_cost_usd: string;
  package_id: string | null;
}

export interface BrandItem {
  id: string;
  name: string;
  client_id: string;
  created_at: string;
}

export interface PackageItem {
  id: string;
  version: number;
  brand_id: string;
  platform: string;
  action: string;
  text_preview: string;
  constraints: { source?: string; provenance?: unknown };
  approval_status: string;
  created_at: string;
}

export const api = {
  health: () => req<Health>("/healthz"),
  version: () => req<VersionInfo>("/version"),
  metrics: () => fetch(`${BASE}/metrics`).then((r) => r.text()) as Promise<string>,
  login: (email: string, password: string) =>
    req<{ token: string; agency_id: string; role: string }>("/auth/login", {
      method: "POST",
      body: JSON.stringify({ email, password }),
    }),
  registerAgency: (name: string, ownerEmail: string, ownerPassword: string) =>
    req<{ id: string; name: string; token: string | null }>("/agencies", {
      method: "POST",
      body: JSON.stringify({ name, owner_email: ownerEmail, owner_password: ownerPassword }),
    }),
  me: () => req<Me>("/agencies/me"),
  listTasks: () => req<{ items: TaskItem[] }>("/tasks"),
  listDevices: () => req<{ items: DeviceItem[] }>("/devices"),
  publishTask: (body: {
    brand_id: string;
    platform: string;
    action: string;
    package_id: string;
    package_version: number;
    idempotency_key: string;
  }) =>
    req<PublishTaskResponse>("/tasks/publish", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  taskRoute: (id: string) => req<unknown>(`/tasks/${id}/route`),
  listKillSwitches: () => req<{ items: KillSwitch[] }>("/kill-switches"),
  createKillSwitch: (body: { scope: string; target_id?: string; reason: string }) =>
    req<{ id: string; active: boolean }>("/kill-switches", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  deactivateKillSwitch: (id: string) =>
    req<{ id: string; active: boolean }>(`/kill-switches/${id}`, {
      method: "DELETE",
    }),
  importMetrics: (body: {
    platform: string;
    external_content_id: string;
    metric_date: string;
    metrics: unknown;
    source: "api" | "device" | "manual";
  }) => req<{ id: string }>("/analytics/metrics", { method: "POST", body: JSON.stringify(body) }),
  aiGenerate: (body: {
    prompt: string;
    max_tokens?: number;
    model?: string;
    save_as_package?: { brand_id: string; platform: string; action: string };
  }) => req<AiGenerateResponse>("/ai/generate", { method: "POST", body: JSON.stringify(body) }),
  listBrands: () => req<{ items: BrandItem[] }>("/brands"),
  listPackages: (platform?: string, action?: string) =>
    req<{ items: PackageItem[] }>(
      "/packages" + (platform ? `?platform=${platform}${action ? `&action=${action}` : ""}` : ""),
    ),
  createPackage: (body: { brand_id: string; platform: string; action: string; text: string }) =>
    req<{ package_id: string; version: number; approval_status: string }>("/packages", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  approvePackage: (id: string) =>
    req<{ id: string; approval_status: string }>(`/packages/${id}/approve`, { method: "POST" }),
};
