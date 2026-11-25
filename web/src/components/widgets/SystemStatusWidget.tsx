import { A } from "@solidjs/router";
import {
  createEffect,
  createMemo,
  createSignal,
  Show,
  type Component,
} from "solid-js";
import { api, type SystemStatusWidgetConfig } from "../../lib/api";
import { useXiaohe } from "../../lib/store";

export const SystemStatusWidget: Component<{
  config: SystemStatusWidgetConfig;
}> = (props) => {
  const store = useXiaohe();
  const [skillsSummary, setSkillsSummary] = createSignal<{
    running: number;
    total: number;
  } | null>(null);
  const [autosSummary, setAutosSummary] = createSignal<{
    enabled: number;
    total: number;
  } | null>(null);

  createEffect(() => {
    if (!store.isAdmin()) return;
    let cancelled = false;
    void Promise.allSettled([api.listSkills(), api.listAutomations()]).then(
      ([skillsRes, autosRes]) => {
        if (cancelled) return;
        if (skillsRes.status === "fulfilled") {
          const list = skillsRes.value.skills;
          const running = list.filter((s) => s.status === "running").length;
          setSkillsSummary({ running, total: list.length });
        }
        if (autosRes.status === "fulfilled") {
          const list = autosRes.value.automations;
          const enabled = list.filter((a) => a.enabled).length;
          setAutosSummary({ enabled, total: list.length });
        }
      },
    );
    return () => {
      cancelled = true;
    };
  });

  const stats = createMemo(() => {
    const list = Object.values(store.entities);
    const online = list.filter((e) => e.availability === "online").length;
    const offline = list.length - online;
    return { total: list.length, online, offline };
  });

  return (
    <div class="h-full rounded-2xl border border-slate-800 bg-slate-900/80 p-5">
      <div class="flex items-center justify-between gap-2">
        <div class="text-sm font-semibold text-white">
          {props.config.title || "System Overview"}
        </div>
        <span class="inline-flex items-center gap-1.5 rounded-full border border-slate-700/80 bg-slate-950/70 px-2.5 py-0.5 text-[11px] text-slate-300">
          <span
            class={`h-2 w-2 rounded-full ${
              store.sseStatus() === "connected"
                ? "bg-emerald-400"
                : "bg-amber-400 animate-pulse"
            }`}
          />
          <span class="capitalize">SSE {store.sseStatus()}</span>
        </span>
      </div>

      <div class="mt-4 grid grid-cols-2 gap-3 sm:grid-cols-4">
        <div class="rounded-xl border border-slate-800 bg-slate-950/60 p-3">
          <div class="text-[10px] uppercase tracking-wider text-slate-400">
            Online Entities
          </div>
          <div class="mt-1 flex items-baseline gap-1.5">
            <span class="text-2xl font-bold text-emerald-300 tabular-nums">
              {stats().online}
            </span>
            <span class="text-xs text-slate-500">/ {stats().total}</span>
          </div>
        </div>

        <div class="rounded-xl border border-slate-800 bg-slate-950/60 p-3">
          <div class="text-[10px] uppercase tracking-wider text-slate-400">
            Areas
          </div>
          <div class="mt-1 text-2xl font-bold text-white tabular-nums">
            {store.areas().length}
          </div>
        </div>

        <Show
          when={skillsSummary()}
          fallback={
            <div class="rounded-xl border border-slate-800 bg-slate-950/60 p-3">
              <div class="text-[10px] uppercase tracking-wider text-slate-400">
                Offline
              </div>
              <div class="mt-1 text-2xl font-bold text-slate-300 tabular-nums">
                {stats().offline}
              </div>
            </div>
          }
        >
          {(sk) => (
            <A
              href="/skills"
              class="rounded-xl border border-slate-800 bg-slate-950/60 p-3 transition hover:border-slate-700"
            >
              <div class="text-[10px] uppercase tracking-wider text-slate-400">
                Skills Running
              </div>
              <div class="mt-1 flex items-baseline gap-1.5">
                <span class="text-2xl font-bold text-sky-300 tabular-nums">
                  {sk().running}
                </span>
                <span class="text-xs text-slate-500">/ {sk().total}</span>
              </div>
            </A>
          )}
        </Show>

        <Show
          when={autosSummary()}
          fallback={
            <div class="rounded-xl border border-slate-800 bg-slate-950/60 p-3">
              <div class="text-[10px] uppercase tracking-wider text-slate-400">
                Role
              </div>
              <div class="mt-1 text-lg font-semibold capitalize text-white">
                {store.user()?.role ?? "—"}
              </div>
            </div>
          }
        >
          {(au) => (
            <A
              href="/automations"
              class="rounded-xl border border-slate-800 bg-slate-950/60 p-3 transition hover:border-slate-700"
            >
              <div class="text-[10px] uppercase tracking-wider text-slate-400">
                Automations
              </div>
              <div class="mt-1 flex items-baseline gap-1.5">
                <span class="text-2xl font-bold text-amber-300 tabular-nums">
                  {au().enabled}
                </span>
                <span class="text-xs text-slate-500">/ {au().total}</span>
              </div>
            </A>
          )}
        </Show>
      </div>
    </div>
  );
};
