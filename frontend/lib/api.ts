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
  text_content: string;
  constraints: { source?: string; provenance?: unknown };
  approval_status: string;
  created_at: string;
}


export interface ClientItem {
  id: string;
  name: string;
  status: string;
  created_at: string;
}

export interface EmployeeItem {
  id: string;
  name: string;
  role: string;
  status: string;
  channel: string;
  auto_publish: boolean;
  brand_id: string | null;
  config_version: number;
}

export interface CycleItem {
  id: string;
  state: string;
  trigger: string;
  summary: Record<string, unknown> | null;
  started_at: string | null;
  finished_at: string | null;
  created_at: string;
}

export interface CycleDetail extends CycleItem {
  employee_id: string;
  tasks: { id: string; agent: string; step: string; state: string; output: unknown }[];
}

export interface CycleOutcome {
  cycle_id: string;
  state: string;
  selected_opportunity: string | null;
  package_id: string | null;
  task_id: string | null;
  task_state: string | null;
  summary: Record<string, unknown>;
}

export interface MemoryItem {
  id: string;
  type: string;
  content: string;
  evidence: unknown;
  confidence: string;
  created_at: string;
}

export interface InsightItem {
  id: string;
  cycle_id: string | null;
  content: string;
  evidence: unknown;
  confidence: string;
  created_at: string;
}

export interface IntelSource {
  id: string;
  name: string;
  category: string;
  access_method: string;
  capabilities: string[];
  platforms: string[];
  auth_ref: string | null;
  terms: string | null;
  enabled: boolean;
}

export interface IntelProvider {
  id: string;
  source_id: string;
  name: string;
  endpoint: string | null;
  capabilities: string[];
  platforms: string[];
  cost_model: Record<string, unknown>;
  rate_limit: Record<string, unknown>;
  health: string;
  reliability: string;
  enabled: boolean;
  last_health_at: string | null;
}

export interface IntelItem {
  id: string;
  source_id: string;
  provider_id: string | null;
  kind: string;
  dedup_key: string;
  payload: Record<string, unknown>;
  collected_at: string;
}

export interface IntelJob {
  source_id: string;
  schedule: string;
  status: string;
  last_run_at: string | null;
  last_error: string | null;
  last_inserted: number | null;
}

export interface IntelStats {
  total_items: number;
  by_kind: { kind: string; n: number }[];
  by_source: { source_id: string; n: number; last_at: string | null }[];
  opportunities: number;
  last_pass_inserted_total: number;
  last_run_at: string | null;
}

export interface CollectResult {
  inserted: number;
  duplicates: number;
  failures: string[];
  opportunities: number;
  opportunities_upserted: number;
}

export interface OpportunityItem {
  id: string;
  topic: string;
  score: string;
  rationale: Record<string, unknown>;
  evidence: string[];
  status: string;
  created_at: string;
}


export interface ArchNode {
  id: string;
  title: string;
  tech: string;
  desc: string;
}

export interface ArchLayer {
  id: string;
  title: string;
  nodes: ArchNode[];
}

export interface ArchEdge {
  from: string;
  to: string;
  label: string;
  kind: string;
}

export interface ArchFlow {
  id: string;
  title: string;
  steps: string;
}

export interface Architecture {
  title: string;
  service: string;
  version: string;
  generated_at: string;
  layers: ArchLayer[];
  edges: ArchEdge[];
  flows: ArchFlow[];
  edge_kinds: Record<string, string>;
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
  listClients: () => req<{ items: ClientItem[] }>("/clients"),
  createClient: (name: string) =>
    req<{ id: string; name: string; status: string }>("/clients", {
      method: "POST",
      body: JSON.stringify({ name }),
    }),
  createBrand: (body: { client_id: string; name: string; config?: unknown }) =>
    req<{ id: string; name: string; client_id: string }>("/brands", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  listEmployees: () => req<{ items: EmployeeItem[] }>("/employees"),
  createEmployee: (body: {
    name: string;
    role?: string;
    brand_id?: string;
    channel?: string;
    auto_publish?: boolean;
    identity?: unknown;
    mission?: unknown;
    strategy?: unknown;
  }) => req<{ id: string; status: string }>("/employees", { method: "POST", body: JSON.stringify(body) }),
  patchEmployee: (id: string, body: { name?: string; auto_publish?: boolean }) =>
    req<{ id: string; patched: boolean }>(`/employees/${id}`, {
      method: "PATCH",
      body: JSON.stringify(body),
    }),
  employeeAction: (id: string, action: "start" | "pause" | "resume" | "stop") =>
    req<{ id: string; status: string }>(`/employees/${id}/${action}`, { method: "POST" }),
  listCycles: (id: string, limit = 20) =>
    req<{ items: CycleItem[] }>(`/employees/${id}/cycles?limit=${limit}`),
  runCycle: (id: string, trigger = "manual") =>
    req<CycleOutcome>(`/employees/${id}/cycles`, {
      method: "POST",
      body: JSON.stringify({ trigger }),
    }),
  getCycle: (id: string) => req<CycleDetail>(`/cycles/${id}`),
  listMemory: (id: string) => req<{ items: MemoryItem[] }>(`/employees/${id}/memory`),
  listInsights: (id: string) => req<{ items: InsightItem[] }>(`/employees/${id}/insights`),
  getArchitecture: () => req<Architecture>("/architecture"),
  listIntelSources: () => req<{ items: IntelSource[] }>("/intel/sources"),
  listIntelProviders: () => req<{ items: IntelProvider[] }>("/intel/providers"),
  providerHealth: (id: string) =>
    req<{ id: string; name: string; health: string; latency_ms: number; detail: string }>(
      `/intel/providers/${id}/health`,
      { method: "POST" },
    ),
  intelRoute: (body: { capability: string; platform?: string; max_cost_usd?: number }) =>
    req<{ chain: string[]; providers: { id: string; name: string }[]; reason: string }>(
      "/intel/route",
      { method: "POST", body: JSON.stringify(body) },
    ),
  listIntelItems: (kind?: string, limit = 50) =>
    req<{ items: IntelItem[] }>("/intel/items" + (kind ? `?kind=${kind}&limit=${limit}` : `?limit=${limit}`)),
  listOpportunities: (limit = 50) =>
    req<{ items: OpportunityItem[] }>(`/intel/opportunities?limit=${limit}`),
  listIntelJobs: () => req<{ items: IntelJob[] }>("/intel/jobs"),
  intelStats: () => req<IntelStats>("/intel/stats"),
  collectNow: () =>
    req<CollectResult>("/intel/collect", { method: "POST" }),
  refreshOpportunities: () =>
    req<{ opportunities: number; upserted: number }>("/intel/opportunities", { method: "POST" }),
  approvePackage: (id: string) =>
    req<{ id: string; approval_status: string }>(`/packages/${id}/approve`, { method: "POST" }),
};
