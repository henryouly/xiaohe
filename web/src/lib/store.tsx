import {
  createContext,
  createEffect,
  createSignal,
  onCleanup,
  useContext,
  type Accessor,
  type ParentComponent,
} from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import {
  api,
  ApiError,
  type AreaSummary,
  type AutomationSummary,
  type EntityRecord,
  type SkillSummary,
  type SseAutomationRunPayload,
  type SseEntityStatePayload,
  type SseSkillStatusPayload,
  type UserSummary,
} from "./api";
import { applyEntityStateEvent, mergeEntitiesSnapshot } from "./merge";

export type SseStatus = "disconnected" | "connecting" | "connected" | "reconnecting";

export interface ToastMessage {
  id: number;
  kind: "info" | "success" | "error";
  text: string;
}

interface XiaoheStoreValue {
  initialized: Accessor<boolean>;
  user: Accessor<UserSummary | null>;
  authLoading: Accessor<boolean>;
  sseStatus: Accessor<SseStatus>;
  areas: Accessor<AreaSummary[]>;
  entities: Record<string, EntityRecord>;
  skills: Accessor<SkillSummary[]>;
  automations: Accessor<AutomationSummary[]>;
  toasts: Accessor<ToastMessage[]>;
  isAdmin: Accessor<boolean>;
  refreshAuth: () => Promise<void>;
  refreshSnapshot: () => Promise<void>;
  refreshAdminData: () => Promise<void>;
  bootstrap: (username: string, password: string, displayName: string) => Promise<void>;
  login: (username: string, password: string) => Promise<void>;
  logout: () => Promise<void>;
  notify: (kind: ToastMessage["kind"], text: string) => void;
}

const XiaoheContext = createContext<XiaoheStoreValue>();

export const XiaoheProvider: ParentComponent = (props) => {
  const [initialized, setInitialized] = createSignal(true);
  const [user, setUser] = createSignal<UserSummary | null>(null);
  const [authLoading, setAuthLoading] = createSignal(true);
  const [sseStatus, setSseStatus] = createSignal<SseStatus>("disconnected");
  const [areas, setAreas] = createSignal<AreaSummary[]>([]);
  const [entities, setEntities] = createStore<Record<string, EntityRecord>>({});
  const [skills, setSkills] = createSignal<SkillSummary[]>([]);
  const [automations, setAutomations] = createSignal<AutomationSummary[]>([]);
  const [toasts, setToasts] = createSignal<ToastMessage[]>([]);

  let nextToastId = 1;
  const notify = (kind: ToastMessage["kind"], text: string) => {
    const id = nextToastId++;
    setToasts((prev) => [...prev.slice(-3), { id, kind, text }]);
    window.setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
    }, 4000);
  };

  const isAdmin = () => user()?.role === "admin";

  const clearSessionData = () => {
    setAreas([]);
    setEntities(reconcile({}));
    setSkills([]);
    setAutomations([]);
  };

  const refreshAuth = async () => {
    setAuthLoading(true);
    try {
      const me = await api.me();
      setInitialized(me.initialized);
      setUser(me.user);
      if (!me.user) {
        clearSessionData();
      }
    } catch {
      setUser(null);
      clearSessionData();
    } finally {
      setAuthLoading(false);
    }
  };

  const refreshSnapshot = async () => {
    if (!user()) return;
    try {
      const [areasResp, entitiesResp] = await Promise.all([
        api.listAreas(),
        api.listEntities(),
      ]);
      setAreas(areasResp.areas);
      const merged = mergeEntitiesSnapshot({ ...entities }, entitiesResp.entities);
      setEntities(reconcile(merged));
    } catch (err) {
      if (err instanceof ApiError && err.status === 401) {
        setUser(null);
        clearSessionData();
      }
    }
  };

  const refreshAdminData = async () => {
    if (!isAdmin()) return;
    try {
      const [skillsResp, autosResp] = await Promise.all([
        api.listSkills().catch(() => ({ skills: [] })),
        api.listAutomations().catch(() => ({ automations: [] })),
      ]);
      setSkills(skillsResp.skills);
      setAutomations(autosResp.automations);
    } catch {
      // Non-fatal; individual views surface detailed errors.
    }
  };

  const bootstrap = async (
    username: string,
    password: string,
    displayName: string,
  ) => {
    const resp = await api.bootstrap({
      username,
      password,
      display_name: displayName,
    });
    setInitialized(true);
    setUser(resp.user);
    notify("success", `Welcome, ${resp.user.display_name}!`);
  };

  const login = async (username: string, password: string) => {
    const resp = await api.login({ username, password });
    setInitialized(true);
    setUser(resp.user);
    notify("success", `Signed in as ${resp.user.display_name}`);
  };

  const logout = async () => {
    try {
      await api.logout();
    } finally {
      setUser(null);
      clearSessionData();
      notify("info", "Signed out");
    }
  };

  // Initial auth probe on mount
  void refreshAuth();

  // Manage live SSE subscription & initial snapshot whenever authenticated user changes
  createEffect(() => {
    const currentUser = user();
    if (!currentUser) {
      setSseStatus("disconnected");
      return;
    }

    let closed = false;
    let es: EventSource | null = null;
    let reconnectTimer: number | undefined;
    let backoffMs = 1000;

    const connect = (isReconnect: boolean) => {
      if (closed) return;
      setSseStatus(isReconnect ? "reconnecting" : "connecting");

      es = new EventSource("/api/events");

      es.onopen = () => {
        backoffMs = 1000;
        setSseStatus("connected");
        // Always reconcile snapshot after connecting / reconnecting.
        void refreshSnapshot();
        if (currentUser.role === "admin") {
          void refreshAdminData();
        }
      };

      es.addEventListener("entity_state", (ev) => {
        try {
          const payload = JSON.parse((ev as MessageEvent).data) as SseEntityStatePayload;
          const updated = applyEntityStateEvent(entities[payload.entity_id], payload);
          if (updated) {
            setEntities(payload.entity_id, updated);
          } else {
            // New entity discovered after initial snapshot — resync metadata.
            void refreshSnapshot();
          }
        } catch {
          // Ignore malformed frame
        }
      });

      es.addEventListener("skill_status", (ev) => {
        try {
          const payload = JSON.parse((ev as MessageEvent).data) as SseSkillStatusPayload;
          setSkills((prev) =>
            prev.map((s) =>
              s.skill_id === payload.skill_id
                ? {
                    ...s,
                    status: payload.status,
                    last_error: payload.last_error ?? null,
                  }
                : s,
            ),
          );
        } catch {
          // Ignore malformed frame
        }
      });

      es.addEventListener("automation_run", (ev) => {
        try {
          const payload = JSON.parse((ev as MessageEvent).data) as SseAutomationRunPayload;
          setAutomations((prev) =>
            prev.map((a) =>
              a.id === payload.automation_id
                ? {
                    ...a,
                    last_run_at: payload.finished_at,
                    last_outcome: payload.outcome,
                  }
                : a,
            ),
          );
        } catch {
          // Ignore malformed frame
        }
      });

      es.addEventListener("resync", () => {
        void refreshSnapshot();
        if (currentUser.role === "admin") {
          void refreshAdminData();
        }
      });

      es.onerror = () => {
        if (closed) return;
        es?.close();
        es = null;
        setSseStatus("reconnecting");
        const delay = backoffMs;
        backoffMs = Math.min(backoffMs * 2, 15_000);
        reconnectTimer = window.setTimeout(() => {
          // Verify session wasn't revoked before reconnecting
          void api.me().then((me) => {
            if (!me.user) {
              setUser(null);
              clearSessionData();
              return;
            }
            connect(true);
          }).catch(() => {
            connect(true);
          });
        }, delay);
      };
    };

    connect(false);

    onCleanup(() => {
      closed = true;
      if (reconnectTimer !== undefined) {
        window.clearTimeout(reconnectTimer);
      }
      es?.close();
      setSseStatus("disconnected");
    });
  });

  const value: XiaoheStoreValue = {
    initialized,
    user,
    authLoading,
    sseStatus,
    areas,
    entities,
    skills,
    automations,
    toasts,
    isAdmin,
    refreshAuth,
    refreshSnapshot,
    refreshAdminData,
    bootstrap,
    login,
    logout,
    notify,
  };

  return (
    <XiaoheContext.Provider value={value}>
      {props.children}
    </XiaoheContext.Provider>
  );
};

export function useXiaohe(): XiaoheStoreValue {
  const ctx = useContext(XiaoheContext);
  if (!ctx) {
    throw new Error("useXiaohe must be used inside <XiaoheProvider>");
  }
  return ctx;
}
