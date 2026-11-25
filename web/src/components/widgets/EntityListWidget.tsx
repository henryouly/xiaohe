import { A } from "@solidjs/router";
import { createMemo, createSignal, For, Show, type Component } from "solid-js";
import { api, ApiError, type EntityListWidgetConfig } from "../../lib/api";
import { useXiaohe } from "../../lib/store";
import {
  filterEntitiesForList,
  formatEntityStateSummary,
} from "../../lib/widgets";

export const EntityListWidget: Component<{ config: EntityListWidgetConfig }> = (
  props,
) => {
  const store = useXiaohe();
  const [busyId, setBusyId] = createSignal<string | null>(null);

  const items = createMemo(() =>
    filterEntitiesForList(Object.values(store.entities), {
      area_id: props.config.area_id,
      domain: props.config.domain,
      entity_ids: props.config.entity_ids,
    }),
  );

  const toggleEntity = async (entityId: string, currentlyOn: boolean) => {
    setBusyId(entityId);
    try {
      await api.commandEntity(entityId, currentlyOn ? "turn_off" : "turn_on");
    } catch (err) {
      store.notify(
        "error",
        err instanceof ApiError ? err.message : "Command failed",
      );
    } finally {
      setBusyId(null);
    }
  };

  const subtitle = () => {
    const parts: string[] = [];
    if (props.config.area_id) parts.push(`area:${props.config.area_id}`);
    if (props.config.domain) parts.push(`domain:${props.config.domain}`);
    if (props.config.entity_ids && props.config.entity_ids.length > 0) {
      parts.push(`${props.config.entity_ids.length} pinned`);
    }
    return parts.length > 0 ? parts.join(" · ") : "All accessible entities";
  };

  return (
    <div class="h-full rounded-2xl border border-slate-800 bg-slate-900/80 p-5 flex flex-col gap-3">
      <div class="flex items-center justify-between gap-2">
        <div class="min-w-0">
          <div class="truncate text-sm font-semibold text-white">
            {props.config.title || "Entity States"}
          </div>
          <div class="truncate text-[11px] text-slate-500">{subtitle()}</div>
        </div>
        <span class="rounded-full bg-slate-800 px-2.5 py-0.5 text-[11px] font-medium text-slate-300">
          {items().length}
        </span>
      </div>

      <Show
        when={items().length > 0}
        fallback={
          <div class="rounded-xl border border-dashed border-slate-800 p-6 text-center text-xs text-slate-500">
            No entities match this filter.
          </div>
        }
      >
        <div class="divide-y divide-slate-800/80 overflow-y-auto max-h-72">
          <For each={items()}>
            {(entity) => {
              const isToggleable = () =>
                entity.state.domain === "switch" ||
                entity.state.domain === "light";
              const isOn = () =>
                (entity.state.domain === "switch" ||
                  entity.state.domain === "light") &&
                entity.state.on;

              return (
                <div class="flex items-center justify-between gap-3 py-2.5 first:pt-0 last:pb-0">
                  <div class="flex items-center gap-2.5 min-w-0">
                    <span
                      class={`inline-block h-2 w-2 shrink-0 rounded-full ${
                        entity.availability === "online"
                          ? "bg-emerald-400"
                          : "bg-rose-400"
                      }`}
                    />
                    <div class="min-w-0">
                      <A
                        href={`/history?entity=${encodeURIComponent(entity.entity_id)}`}
                        class="block truncate text-xs font-medium text-slate-100 hover:text-emerald-300"
                      >
                        {entity.name}
                      </A>
                      <div class="truncate text-[10px] font-mono text-slate-500">
                        {entity.entity_id}
                      </div>
                    </div>
                  </div>

                  <div class="flex items-center gap-2 shrink-0">
                    <Show
                      when={isToggleable()}
                      fallback={
                        <span class="rounded-md bg-slate-800/90 px-2 py-0.5 text-xs font-medium text-slate-200 tabular-nums">
                          {formatEntityStateSummary(entity)}
                        </span>
                      }
                    >
                      <button
                        type="button"
                        disabled={
                          busyId() === entity.entity_id ||
                          entity.availability !== "online"
                        }
                        onClick={() =>
                          void toggleEntity(entity.entity_id, isOn())
                        }
                        class={`rounded-full px-3 py-0.5 text-[11px] font-semibold transition ${
                          isOn()
                            ? "bg-emerald-400 text-slate-950 hover:bg-emerald-300"
                            : "bg-slate-800 text-slate-300 hover:bg-slate-700"
                        } disabled:opacity-50`}
                      >
                        {isOn() ? "ON" : "OFF"}
                      </button>
                    </Show>
                  </div>
                </div>
              );
            }}
          </For>
        </div>
      </Show>
    </div>
  );
};
