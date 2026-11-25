// TypeScript mirror of Xiaohe Core REST & SSE API (docs/api.md & src/http/*).

export type UserRole = "admin" | "member" | "guest";

export interface ApiTokenSummary {
  id: string;
  name: string;
  created_at: number;
}

export interface UserSummary {
  id: string;
  username: string;
  display_name: string;
  role: UserRole;
  allowed_areas: string[] | "*";
  allow_guest_locks?: boolean;
  api_tokens?: ApiTokenSummary[];
}

export interface AuthMeResponse {
  initialized: boolean;
  user: UserSummary | null;
}

export interface AreaSummary {
  area_id: string;
  name: string;
  icon?: string;
  entity_count: number;
}

export type Availability = "online" | "offline" | "unknown";

export type ClimateMode = "off" | "heat" | "cool" | "auto" | "dry" | "fan_only";
export type CoverState = "open" | "opening" | "closed" | "closing" | "stopped";
export type LockState = "locked" | "unlocked" | "jammed";
export type MediaState = "playing" | "paused" | "idle" | "off";

export type DomainState =
  | {
      domain: "light";
      on: boolean;
      brightness?: number;
      color_temp_k?: number;
      rgb?: [number, number, number];
    }
  | { domain: "switch"; on: boolean }
  | {
      domain: "climate";
      mode: ClimateMode;
      target_temp_c?: number;
      current_temp_c?: number;
      fan_mode?: string;
    }
  | {
      domain: "sensor";
      value: number | string;
      unit?: string;
      device_class?: string;
    }
  | { domain: "binary_sensor"; active: boolean; device_class?: string }
  | { domain: "cover"; state: CoverState; position?: number }
  | { domain: "lock"; state: LockState }
  | {
      domain: "media_player";
      state: MediaState;
      volume?: number;
      muted?: boolean;
      title?: string;
    }
  | { domain: "button"; last_pressed_at?: number }
  | { domain: "custom"; state: string };

export type EntitySource =
  | { type: "esphome"; node_id: string }
  | { type: "skill"; skill_id: string };

export interface CustomActionDef {
  name: string;
  description?: string;
  params?: Record<string, unknown>;
}

export interface EntityRecord {
  entity_id: string;
  device_id?: string;
  area_id?: string;
  name: string;
  source: EntitySource;
  availability: Availability;
  state: DomainState;
  capabilities?: string[];
  extra?: Record<string, unknown>;
  custom_actions?: CustomActionDef[];
  last_changed_at: number;
  last_updated_at: number;
}

export interface NumericPoint {
  ts: number;
  avg: number;
  min: number;
  max: number;
}

export interface DiscreteTransition {
  ts: number;
  state: string;
}

export type EntityHistorySeries =
  | {
      kind: "numeric";
      entity_id: string;
      unit?: string;
      points: NumericPoint[];
    }
  | {
      kind: "discrete";
      entity_id: string;
      transitions: DiscreteTransition[];
    };

export type SkillMode = "device" | "action";
export type SkillStatus = "stopped" | "installing" | "running" | "error";

export interface SkillSummary {
  skill_id: string;
  name: string;
  version: string;
  mode: SkillMode;
  enabled: boolean;
  status: SkillStatus;
  restart_count: number;
  last_error: string | null;
  entity_count: number;
}

export interface SkillDetailResponse {
  skill_id: string;
  name: string;
  version: string;
  mode: SkillMode;
  enabled: boolean;
  status: SkillStatus;
  restart_count: number;
  last_error: string | null;
  config: Record<string, unknown>;
  secrets: Record<string, { is_set: boolean }>;
  manifest?: Record<string, unknown>;
}

export interface SkillLogEntry {
  ts: number;
  stream: string;
  line: string;
}

export type AutomationOutcome =
  | "succeeded"
  | "skipped"
  | "conditions_not_met"
  | "cancelled"
  | "skipped_single"
  | "failed";

export interface AutomationSummary {
  id: string;
  name: string;
  enabled: boolean;
  mode: string;
  file: string;
  last_run_at: number | null;
  last_outcome: AutomationOutcome | null;
}

export interface AutomationDetail {
  id: string;
  file: string;
  parsed: Record<string, unknown>;
  raw_toml: string;
}

export interface AutomationRunRecord {
  run_id: string;
  automation_id: string;
  trigger_source: string;
  outcome: AutomationOutcome;
  started_at: number;
  finished_at: number | null;
  error: string | null;
}

export interface ValidationErrorDetail {
  line: number;
  message: string;
}

export interface SseEntityStatePayload {
  entity_id: string;
  area_id?: string | null;
  availability?: Availability | null;
  state?: DomainState | null;
  extra?: Record<string, unknown> | null;
  last_changed_at: number;
  last_updated_at: number;
}

export interface SseSkillStatusPayload {
  skill_id: string;
  status: SkillStatus;
  last_error?: string | null;
  at: number;
}

export interface SseAutomationRunPayload {
  run_id: string;
  automation_id: string;
  outcome: AutomationOutcome;
  finished_at: number;
}

export class ApiError extends Error {
  status: number;
  code: string;
  details?: ValidationErrorDetail[];

  constructor(
    status: number,
    code: string,
    message: string,
    details?: ValidationErrorDetail[],
  ) {
    super(message);
    this.status = status;
    this.code = code;
    this.details = details;
  }
}

async function request<T>(
  path: string,
  init: RequestInit = {},
): Promise<T> {
  const headers = new Headers(init.headers);
  if (init.body !== undefined && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }

  const resp = await fetch(path, {
    ...init,
    credentials: "same-origin",
    headers,
  });

  if (resp.status === 204) {
    return undefined as T;
  }

  const text = await resp.text();
  let parsed: Record<string, unknown> | null = null;
  if (text.length > 0) {
    try {
      parsed = JSON.parse(text) as Record<string, unknown>;
    } catch {
      parsed = null;
    }
  }

  if (!resp.ok) {
    const code =
      (typeof parsed?.error === "string" ? parsed.error : undefined) ??
      `http_${resp.status}`;
    const msg =
      (typeof parsed?.message === "string" ? parsed.message : undefined) ??
      code;
    const details = Array.isArray(parsed?.details)
      ? (parsed.details as ValidationErrorDetail[])
      : undefined;
    throw new ApiError(resp.status, code, msg, details);
  }

  return parsed as T;
}

export const api = {
  // Auth
  me: () => request<AuthMeResponse>("/api/auth/me"),
  bootstrap: (body: {
    username: string;
    password: string;
    display_name: string;
  }) =>
    request<{ user: UserSummary }>("/api/auth/bootstrap", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  login: (body: { username: string; password: string }) =>
    request<{ user: UserSummary }>("/api/auth/login", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  logout: () => request<void>("/api/auth/logout", { method: "POST" }),

  // Areas
  listAreas: () => request<{ areas: AreaSummary[] }>("/api/areas"),
  createArea: (body: { area_id: string; name: string; icon?: string }) =>
    request<{ area: AreaSummary }>("/api/areas", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  deleteArea: (areaId: string) =>
    request<void>(`/api/areas/${encodeURIComponent(areaId)}`, {
      method: "DELETE",
    }),

  // Entities & History
  listEntities: (params?: {
    area_id?: string;
    domain?: string;
    source?: string;
  }) => {
    const qs = new URLSearchParams();
    if (params?.area_id) qs.set("area_id", params.area_id);
    if (params?.domain) qs.set("domain", params.domain);
    if (params?.source) qs.set("source", params.source);
    const suffix = qs.toString() ? `?${qs.toString()}` : "";
    return request<{ entities: EntityRecord[] }>(`/api/entities${suffix}`);
  },
  commandEntity: (
    entityId: string,
    action: string,
    params: Record<string, unknown> = {},
  ) =>
    request<{ ok: boolean; entity_id: string }>(
      `/api/entities/${encodeURIComponent(entityId)}/command`,
      {
        method: "POST",
        body: JSON.stringify({ action, params }),
      },
    ),
  entityHistory: (
    entityId: string,
    params?: { from?: number; to?: number; bucket_secs?: number },
  ) => {
    const qs = new URLSearchParams();
    if (params?.from !== undefined) qs.set("from", String(params.from));
    if (params?.to !== undefined) qs.set("to", String(params.to));
    if (params?.bucket_secs !== undefined)
      qs.set("bucket_secs", String(params.bucket_secs));
    const suffix = qs.toString() ? `?${qs.toString()}` : "";
    return request<EntityHistorySeries>(
      `/api/entities/${encodeURIComponent(entityId)}/history${suffix}`,
    );
  },

  // Skills (Admin)
  listSkills: () => request<{ skills: SkillSummary[] }>("/api/skills"),
  getSkill: (skillId: string) =>
    request<SkillDetailResponse>(`/api/skills/${encodeURIComponent(skillId)}`),
  updateSkillConfig: (
    skillId: string,
    body: {
      config?: Record<string, unknown>;
      secrets?: Record<string, string>;
      enabled?: boolean;
    },
  ) =>
    request<{ ok: boolean; reloaded: boolean }>(
      `/api/skills/${encodeURIComponent(skillId)}/config`,
      {
        method: "PUT",
        body: JSON.stringify(body),
      },
    ),
  skillLifecycle: (
    skillId: string,
    op: "start" | "stop" | "restart" | "reinstall",
  ) =>
    request<{ skill_id: string; status: SkillStatus }>(
      `/api/skills/${encodeURIComponent(skillId)}/${op}`,
      { method: "POST" },
    ),
  skillLogs: (skillId: string, tail = 200) =>
    request<{ skill_id: string; lines: SkillLogEntry[] }>(
      `/api/skills/${encodeURIComponent(skillId)}/logs?tail=${tail}`,
    ),
  invokeSkill: (
    skillId: string,
    action: string,
    params: Record<string, unknown> = {},
  ) =>
    request<{ ok: boolean; result: unknown }>(
      `/api/skills/${encodeURIComponent(skillId)}/invoke`,
      {
        method: "POST",
        body: JSON.stringify({ action, params }),
      },
    ),

  // Automations (Admin)
  listAutomations: () =>
    request<{ automations: AutomationSummary[] }>("/api/automations"),
  getAutomation: (id: string) =>
    request<AutomationDetail>(`/api/automations/${encodeURIComponent(id)}`),
  createAutomation: (raw_toml: string) =>
    request<{ id: string; warnings: string[] }>("/api/automations", {
      method: "POST",
      body: JSON.stringify({ raw_toml }),
    }),
  updateAutomation: (id: string, raw_toml: string) =>
    request<{ id: string; warnings: string[] }>(
      `/api/automations/${encodeURIComponent(id)}`,
      {
        method: "PUT",
        body: JSON.stringify({ raw_toml }),
      },
    ),
  toggleAutomation: (id: string, enabled: boolean) =>
    request<{ id: string; enabled: boolean }>(
      `/api/automations/${encodeURIComponent(id)}/toggle`,
      {
        method: "POST",
        body: JSON.stringify({ enabled }),
      },
    ),
  triggerAutomation: (id: string, skip_conditions = false) =>
    request<{ run_id: string; outcome: string }>(
      `/api/automations/${encodeURIComponent(id)}/trigger`,
      {
        method: "POST",
        body: JSON.stringify({ skip_conditions }),
      },
    ),
  deleteAutomation: (id: string) =>
    request<void>(`/api/automations/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),
  reloadAutomations: () =>
    request<{ loaded: number; errors: string[] }>("/api/automations/reload", {
      method: "POST",
    }),
  listAutomationRuns: (params?: {
    automation_id?: string;
    limit?: number;
    offset?: number;
  }) => {
    const qs = new URLSearchParams();
    if (params?.automation_id) qs.set("automation_id", params.automation_id);
    if (params?.limit !== undefined) qs.set("limit", String(params.limit));
    if (params?.offset !== undefined) qs.set("offset", String(params.offset));
    const suffix = qs.toString() ? `?${qs.toString()}` : "";
    return request<{ runs: AutomationRunRecord[] }>(
      `/api/automations/runs${suffix}`,
    );
  },

  // Dashboard Layout
  getDashboard: () => request<DashboardConfig>("/api/dashboard"),
  saveDashboard: (config: DashboardConfig) =>
    request<DashboardConfig>("/api/dashboard", {
      method: "PUT",
      body: JSON.stringify(config),
    }),
};

export type WidgetSize = "sm" | "md" | "lg";

export type WidgetAccent = "emerald" | "sky" | "amber" | "rose" | "violet";

export type DashboardWidgetBase = {
  id: string;
  title?: string;
  size: WidgetSize;
};

export type MetricWidgetConfig = DashboardWidgetBase & {
  type: "metric";
  entity_id: string;
  accent?: WidgetAccent;
  warn_above?: number;
  crit_above?: number;
};

export type SparklineWidgetConfig = DashboardWidgetBase & {
  type: "sparkline";
  entity_id: string;
  window_hours?: number;
  accent?: WidgetAccent;
};

export type EntityCardWidgetConfig = DashboardWidgetBase & {
  type: "entity_card";
  entity_id: string;
};

export type WeatherWidgetConfig = DashboardWidgetBase & {
  type: "weather";
  prefix?: string;
};

export type EntityListWidgetConfig = DashboardWidgetBase & {
  type: "entity_list";
  area_id?: string;
  domain?: string;
  entity_ids?: string[];
};

export type AreaGroupWidgetConfig = DashboardWidgetBase & {
  type: "area_group";
  area_id?: string;
};

export type SystemStatusWidgetConfig = DashboardWidgetBase & {
  type: "system_status";
};

export type DashboardWidget =
  | MetricWidgetConfig
  | SparklineWidgetConfig
  | EntityCardWidgetConfig
  | WeatherWidgetConfig
  | EntityListWidgetConfig
  | AreaGroupWidgetConfig
  | SystemStatusWidgetConfig;

export type WidgetType = DashboardWidget["type"];

export interface DashboardConfig {
  widgets: DashboardWidget[];
}

