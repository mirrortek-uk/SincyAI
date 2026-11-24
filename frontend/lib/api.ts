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
  language: string;
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
  language?: string;
}

export const LANGUAGES: { code: string; label: string }[] = [
  { code: "zh-CN", label: "简体中文 (zh-CN)" },
  { code: "zh-TW", label: "繁體中文 (zh-TW)" },
  { code: "en-US", label: "English (en-US)" },
  { code: "ja-JP", label: "日本語 (ja-JP)" },
  { code: "ko-KR", label: "한국어 (ko-KR)" },
  { code: "fr-FR", label: "Français (fr-FR)" },
  { code: "de-DE", label: "Deutsch (de-DE)" },
  { code: "es-ES", label: "Español (es-ES)" },
  { code: "pt-BR", label: "Português (pt-BR)" },
];

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
  industries?: string[];
  tags?: Record<string, unknown>;
  relevance_score?: number | null;  // entity 层不再写恒 0, per-brand 相关性在 intel_entity_brand_relevance
}

export interface IndustryDef {
  id: string;
  name: string;
  parent_id: string | null;
  level: number;
  keywords: string[];
  description: string;
}

export interface IntelSignal {
  signal_id: string;
  signal_type: string;
  entity_id: string | null;
  source_id: string;
  strength: number;
  importance: number;
  observed_at: string;
  payload: Record<string, unknown>;
}

export interface IntelPipeline {
  id: string;
  input_record_type: string;
  processors: string[];
  output_entity_types: string[];
  bus_subject: string;
  enabled: boolean;
}

export interface IntelConnectorSource {
  source_id: string;
  label: string;
  url: string;
  kind: string;
  record_type: string;
  enabled: boolean;
}

export interface IntelConnector {
  id: string;
  name: string;
  connector_type: string;
  capabilities: string[];
  datasets: { id: string; record_type: string; url?: string | null }[];
  sources: IntelConnectorSource[];
  source_count: number;
  fetch_impl: string;
  default_interval_secs: number;
  enabled: boolean;
  updated_at: string;
}

export interface IntelProcessor {
  processor_id: string;
  version: string;
  input_types: string[];
}

export interface CustomerIntelProfile {
  industry?: string;
  sub_industry?: string;
  business_model?: string;
  target_audience?: string;
  geography?: string[];
  competitors?: string[];
  products?: string[];
  topics?: string[];
  keywords?: string[];
  business_goals?: string[];
  brand_strategy?: string;
}

export interface IntelJob {
  source_id: string;
  schedule: string;
  status: string;
  last_run_at: string | null;
  last_error: string | null;
  last_inserted: number | null;
  last_fetched?: number | null;
  last_new?: number | null;
}

export interface IntelSourceHealth {
  source_id: string;
  name: string;
  category: string;
  enabled: boolean;
  health: "healthy" | "degraded" | "failed" | "not_configured" | "disabled";
  jobs: number;
  jobs_ran: number;
  items_total: number;
  last_new: number | null;
  last_error: string | null;
}

export interface IntelHealth {
  generated_at: string;
  summary: Record<string, number>;
  sources: IntelSourceHealth[];
}

export interface IntelSourceStatus {
  source_id: string;
  name: string;
  category: string;
  class: "A" | "B" | "C";
  health: string;
  items_total: number;
  connector: string | null;
  connector_type: string | null;
  parse_path: string | null;
  missing: string | null;
}

export interface IntelSourceStatusResp {
  summary: { A: number; B: number; C: number };
  sources: IntelSourceStatus[];
}

export interface CustomFeed {
  id: string;
  label: string;
  url: string;
  origin: string | null;
  enabled: boolean;
  created_at: string;
  items_total: number;
}

export interface FeedImportResult {
  list_url: string;
  apply: boolean;
  found: number;
  added: number;
  skipped_existing: number;
  feeds: string[];
}

export interface IntelTrend {
  topic: string;
  total_24h: number;
  last_6h: number;
  velocity_per_h: number;
  growth_pct: number;
  platforms: number;
  sources: string[];
  samples: string[];
}

export interface IntelTrends {
  generated_at: string;
  window_hours: number;
  trends: IntelTrend[];
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
  planned?: boolean;
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

export interface ArchAgent {
  id: string;
  name: string;
  name_zh: string;
  role: string;
  steps: string[];
  domains: string[];
}

export interface ArchSkillDomain {
  id: string;
  title_zh?: string;
  count: number;
  skills: string[];
}

export interface ArchRuntimeStep {
  agent: string;
  step: string;
  state: string;
  skills?: unknown;
  skill?: unknown;
  created_at: string;
}

export interface ArchRuntime {
  cycle_id: string;
  employee_id: string;
  employee_name: string;
  state: string;
  trigger: string;
  created_at: string;
  steps: ArchRuntimeStep[];
}

export interface Architecture {
  title: string;
  service: string;
  version: string;
  generated_at: string;
  layers: ArchLayer[];
  edges: ArchEdge[];
  flows: ArchFlow[];
  agents: ArchAgent[];
  cycle_steps: string[];
  skill_domains: { domains: ArchSkillDomain[]; total_skills: number };
  runtime: ArchRuntime | null;
  edge_kinds: Record<string, string>;
}

export interface SkillItem {
  id: string;
  domain: string;
  name: string;
  version: number;
  title: string;
  description: string;
  tools: string[];
  status: string;
  enabled?: boolean;
  bound_at?: string;
}

export interface PluginAgent {
  type: "agent";
  kind: string;
  title: string;
  description: string;
}

export interface PluginSkill extends SkillItem {
  type: "skill";
  what_it_does?: string | null;
  handler?: boolean;
}

export interface PluginManifest {
  employee: { id: string; name: string; role: string; status: string };
  agents: PluginAgent[];
  skills: PluginSkill[];
}

export interface PulseProvider {
  id: string;
  name: string;
  platform: string;
  category: string;
  legal_status: string;
  reliability: number;
  enabled: boolean;
  schedulable: boolean;
  manifest: {
    capabilities: string[];
    datasets: { id: string; update_frequency_secs: number; entity_type: string; feed_label?: string }[];
    acquisition: { type: string; strategy: string };
  };
  recent_runs?: { dataset_id: string; status: string; items_fetched: number; items_new: number; error: string | null; started_at: string }[];
  created_at: string;
  updated_at: string;
}

export interface PulseEntity {
  id: string;
  entity_type: string;
  platform: string;
  canonical_key: string;
  title: string;
  payload: Record<string, unknown>;
  metrics: Record<string, unknown>;
  trend: { velocity?: number; classification?: string };
  confidence: { score?: number; level?: string };
  updated_at: string;
}

export interface PulseObservation {
  provider_id: string;
  dataset_id: string;
  metric: string;
  value: unknown;
  observed_at: string;
}

export interface PulseHealth {
  providers: { provider_id: string; name: string; legal_status: string; enabled: boolean; status: string; runs_24h: number; ok_rate: number; last_run_at: string | null; last_error: string | null }[];
  entities_total: number;
  observations_total: number;
}

export interface ReviewRule {
  platform: string;
  weighted_limit: number;
  on_over_limit: "reject" | "thread";
  note: string;
  updated_at: string;
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
  retryTask: (id: string) =>
    req<{ task_id: string; state: string; attempt: number }>(`/tasks/${id}/retry`, { method: "POST" }),
  listDevices: () => req<{ items: DeviceItem[] }>("/devices"),
  runSetup: (action: string, app?: string, goal?: string) =>
    req<{ task_id: string; state: string; route: string; adapter?: string }>("/setup/run", {
      method: "POST",
      body: JSON.stringify({ action, ...(app ? { app } : {}), ...(goal ? { goal } : {}) }),
    }),
  listSetupApps: () =>
    req<{
      items: {
        app_key: string;
        display_name: string;
        target_package: string;
        download_url?: string;
        sha256?: string;
      }[];
    }>("/setup/apps"),
  updateSetupApp: (appKey: string, downloadUrl?: string, sha256?: string) =>
    req<{ app_key: string }>(`/setup/apps/${appKey}`, {
      method: "PATCH",
      body: JSON.stringify({ download_url: downloadUrl, sha256 }),
    }),
  createPairingCode: (ttlMinutes?: number) =>
    req<{ code: string; expires_at: string }>("/devices/pairing-codes", {
      method: "POST",
      body: JSON.stringify(ttlMinutes != null ? { ttl_minutes: ttlMinutes } : {}),
    }),
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
  createBrand: (body: { client_id: string; name: string; config?: unknown; language?: string }) =>
    req<{ id: string; name: string; client_id: string; language: string }>("/brands", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  updateBrand: (id: string, body: { name?: string; config?: unknown; language?: string }) =>
    req<BrandItem>(`/brands/${id}`, {
      method: "PATCH",
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
  patchEmployee: (id: string, body: { name?: string; auto_publish?: boolean; language?: string }) =>
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
  listSkills: (q?: { domain?: string; q?: string }) =>
    req<{ items: SkillItem[] }>("/skills" + (q && (q.domain || q.q) ? `?${new URLSearchParams({ ...(q.domain ? { domain: q.domain } : {}), ...(q.q ? { q: q.q } : {}) })}` : "")),
  listSkillDomains: () => req<{ domains: ArchSkillDomain[]; total_skills: number }>("/skills/domains"),
  registerSkill: (body: { domain: string; name: string; title?: string; description?: string; tools?: string[] }) =>
    req<{ id: string; name: string; domain: string; version: number }>("/skills", { method: "POST", body: JSON.stringify(body) }),
  listEmployeeSkills: (id: string) => req<{ items: SkillItem[] }>(`/employees/${id}/skills`),
  listPlugins: (id: string) => req<PluginManifest>(`/employees/${id}/plugins`),
  bindSkill: (id: string, body: { skill_id?: string; name?: string; enabled?: boolean }) =>
    req<{ employee_id: string; skill_id: string; enabled: boolean }>(`/employees/${id}/skills`, { method: "POST", body: JSON.stringify(body) }),
  unbindSkill: (id: string, skillId: string) =>
    req<{ unbound: boolean }>(`/employees/${id}/skills/${skillId}`, { method: "DELETE" }),
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
  listIntelItems: (kind?: string, limit = 50, industry?: string) => {
    const q = new URLSearchParams();
    if (kind) q.set("kind", kind);
    if (industry) q.set("industry", industry);
    q.set("limit", String(limit));
    return req<{ items: IntelItem[] }>("/intel/items?" + q.toString());
  },
  listIndustries: () => req<{ items: IndustryDef[] }>("/industries"),
  createIndustry: (body: Partial<IndustryDef>) =>
    req<{ id: string }>("/industries", { method: "POST", body: JSON.stringify(body) }),
  getIntelProfile: (brandId: string) =>
    req<{ brand_id: string; profile: CustomerIntelProfile }>(`/brands/${brandId}/intel-profile`),
  putIntelProfile: (brandId: string, profile: CustomerIntelProfile) =>
    req<{ brand_id: string; profile: CustomerIntelProfile }>(`/brands/${brandId}/intel-profile`, {
      method: "PUT",
      body: JSON.stringify(profile),
    }),
  listIntelSignals: (type?: string, limit = 50) =>
    req<{ items: IntelSignal[] }>("/intel/signals" + (type ? `?type=${type}&limit=${limit}` : `?limit=${limit}`)),
  listIntelPipelines: () =>
    req<{ pipelines: IntelPipeline[]; processors: IntelProcessor[] }>("/intel/pipelines"),
  listIntelConnectors: () =>
    req<{ connectors: IntelConnector[]; total: number }>("/intel/connectors"),
  getIntelConnector: (id: string) =>
    req<IntelConnector>(`/intel/connectors/${encodeURIComponent(id)}`),
  brandIntelFeed: (brandId: string, limit = 50, minScore?: number) => {
    const q = new URLSearchParams({ limit: String(limit) });
    if (minScore !== undefined) q.set("min_score", String(minScore));
    return req<{ brand_id: string; profile_industries: string[]; count: number; items: IntelItem[] }>(
      `/brands/${brandId}/intel-feed?` + q.toString(),
    );
  },
  listOpportunities: (limit = 50) =>
    req<{ items: OpportunityItem[] }>(`/intel/opportunities?limit=${limit}`),
  listIntelJobs: () => req<{ items: IntelJob[] }>("/intel/jobs"),
  intelStats: () => req<IntelStats>("/intel/stats"),
  intelHealth: () => req<IntelHealth>("/intel/health"),
  intelSourceStatus: () => req<IntelSourceStatusResp>("/intel/sources/status"),
  listCustomFeeds: () => req<{ feeds: CustomFeed[] }>("/intel/feeds"),
  importFeedList: (url: string, apply: boolean) =>
    req<FeedImportResult>("/intel/feeds/import", { method: "POST", body: JSON.stringify({ url, apply }) }),
  toggleIntelSource: (id: string, enabled: boolean) =>
    req<{ id: string; enabled: boolean }>(`/intel/sources/${id}/toggle`, { method: "POST", body: JSON.stringify({ enabled }) }),
  enableAllIntelSources: () => req<{ enabled: number }>("/intel/sources/enable-all", { method: "POST" }),
  resumeAllIntelJobs: () => req<{ resumed: number }>("/intel/jobs/resume-all", { method: "POST" }),
  listReviewRules: () => req<{ items: ReviewRule[] }>("/settings/platform-review-rules"),
  listPulseProviders: () => req<{ items: PulseProvider[] }>("/pulse/providers"),
  getPulseProvider: (id: string) => req<PulseProvider>(`/pulse/providers/${encodeURIComponent(id)}`),
  registerPulseProvider: (manifest: Record<string, unknown>) =>
    req<{ id: string; enabled: boolean; legal_status: string }>("/pulse/providers", { method: "POST", body: JSON.stringify(manifest) }),
  updatePulseProvider: (id: string, body: Record<string, unknown>) =>
    req<{ id: string }>(`/pulse/providers/${encodeURIComponent(id)}`, { method: "PUT", body: JSON.stringify(body) }),
  togglePulseProvider: (id: string, enabled: boolean) =>
    req<{ id: string; enabled: boolean }>(`/pulse/providers/${encodeURIComponent(id)}/toggle`, { method: "POST", body: JSON.stringify({ enabled }) }),
  runPulseProvider: (id: string) =>
    req<{ provider_id: string; fetched: number; new: number; errors: string[] }>(`/pulse/providers/${encodeURIComponent(id)}/run`, { method: "POST" }),
  listPulseEntities: (q?: { type?: string; q?: string; classification?: string; limit?: number }) =>
    req<{ items: PulseEntity[] }>("/pulse/entities" + (q ? `?${new URLSearchParams(Object.fromEntries(Object.entries(q).filter(([, v]) => v !== undefined && v !== "").map(([k, v]) => [k, String(v)])))}` : "")),
  listPulseObservations: (entityId: string) => req<{ items: PulseObservation[] }>(`/pulse/entities/${entityId}/observations`),
  pulseHealth: () => req<PulseHealth>("/pulse/health"),
  upsertReviewRule: (platform: string, body: { weighted_limit: number; on_over_limit: string; note?: string }) =>
    req<ReviewRule>(`/settings/platform-review-rules/${encodeURIComponent(platform)}`, {
      method: "PUT",
      body: JSON.stringify(body),
    }),
  intelTrends: () => req<IntelTrends>("/intel/trends"),
  collectNow: () =>
    req<CollectResult>("/intel/collect", { method: "POST" }),
  refreshOpportunities: () =>
    req<{ opportunities: number; upserted: number }>("/intel/opportunities", { method: "POST" }),
  approvePackage: (id: string) =>
    req<{ id: string; approval_status: string }>(`/packages/${id}/approve`, { method: "POST" }),
};
