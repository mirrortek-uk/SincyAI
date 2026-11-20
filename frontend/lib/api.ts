// Typed client for the Sincy Cloud API (/api/v1). Base URL via
// NEXT_PUBLIC_API_BASE (defaults to the local dev server).

const BASE =
  process.env.NEXT_PUBLIC_API_BASE ?? "http://127.0.0.1:18080/api/v1";

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
  const res = await fetch(`${BASE}${path}`, {
    ...init,
    headers: { "content-type": "application/json", ...init?.headers },
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

export interface PublishTaskResponse {
  task_id: string;
  route: string;
  status: string;
  external_content_id: string | null;
  receipt: unknown;
  fallback: string | null;
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

export const api = {
  health: () => req<Health>("/healthz"),
  version: () => req<VersionInfo>("/version"),
  metrics: () =>
    fetch(`${BASE}/metrics`).then((r) => r.text()) as Promise<string>,
  publishTask: (body: {
    agency_id: string;
    brand_id: string;
    platform: string;
    action: string;
    package_id: string;
    package_version: number;
    idempotency_key: string;
  }) => req<PublishTaskResponse>("/tasks/publish", { method: "POST", body: JSON.stringify(body) }),
  taskRoute: (id: string) => req<unknown>(`/tasks/${id}/route`),
  listKillSwitches: (agencyId?: string) =>
    req<{ items: KillSwitch[] }>(
      `/kill-switches${agencyId ? `?agency_id=${agencyId}` : ""}`,
    ),
  createKillSwitch: (body: {
    agency_id?: string;
    scope: string;
    target_id?: string;
    reason: string;
  }) =>
    req<{ id: string; active: boolean }>("/kill-switches", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  deactivateKillSwitch: (id: string) =>
    req<{ id: string; active: boolean }>(`/kill-switches/${id}`, {
      method: "DELETE",
    }),
};
