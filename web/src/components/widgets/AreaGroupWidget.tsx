import { createMemo, createSignal, For, Show, type Component } from "solid-js";
import type { AreaGroupWidgetConfig, EntityRecord } from "../../lib/api";
import { useXiaohe } from "../../lib/store";
import { EntityCard } from "../EntityCard";

export const AreaGroupWidget: Component<{ config: AreaGroupWidgetConfig }> = (
  props,
) => {
  const store = useXiaohe();
  const [selectedArea, setSelectedArea] = createSignal<string>("__all__");

  const fixedAreaId = () => props.config.area_id;
  const entityList = createMemo(() => Object.values(store.entities));
  const areaNameById = createMemo(() => {
    const map = new Map<string, string>();
    for (const a of store.areas()) map.set(a.area_id, a.name);
    return map;
  });

  const groupedEntities = createMemo(() => {
    const targetArea = fixedAreaId() ?? selectedArea();
    const filtered =
      targetArea === "__all__"
        ? entityList()
        : entityList().filter((e) => (e.area_id ?? "__unassigned__") === targetArea);

    const groups = new Map<string, EntityRecord[]>();
    for (const entity of filtered) {
      const key = entity.area_id ?? "__unassigned__";
      const list = groups.get(key) ?? [];
      list.push(entity);
      groups.set(key, list);
    }
    for (const list of groups.values()) {
      list.sort((a, b) => a.name.localeCompare(b.name));
    }
    return Array.from(groups.entries()).sort(([a], [b]) => {
      if (a === "__unassigned__") return 1;
      if (b === "__unassigned__") return -1;
      return a.localeCompare(b);
    });
  });

  const heading = () => {
    if (props.config.title) return props.config.title;
    const fixed = fixedAreaId();
    if (fixed) return areaNameById().get(fixed) ?? fixed;
    return "Devices & Sensors";
  };

  return (
    <div class="rounded-2xl border border-slate-800/90 bg-slate-900/40 p-5 space-y-5">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div class="text-sm font-semibold text-white">{heading()}</div>
        <Show when={!fixedAreaId()}>
          <div class="flex flex-wrap gap-1.5">
            <button
              type="button"
              onClick={() => setSelectedArea("__all__")}
              class={`rounded-full px-3 py-1 text-xs font-medium transition ${
                selectedArea() === "__all__"
                  ? "bg-emerald-500 text-slate-950"
                  : "bg-slate-900 text-slate-300 border border-slate-800 hover:border-slate-700"
              }`}
            >
              All ({entityList().length})
            </button>
            <For each={store.areas()}>
              {(area) => (
                <button
                  type="button"
                  onClick={() => setSelectedArea(area.area_id)}
                  class={`rounded-full px-3 py-1 text-xs font-medium transition ${
                    selectedArea() === area.area_id
                      ? "bg-emerald-500 text-slate-950"
                      : "bg-slate-900 text-slate-300 border border-slate-800 hover:border-slate-700"
                  }`}
                >
                  {area.name} ({area.entity_count})
                </button>
              )}
            </For>
          </div>
        </Show>
      </div>

      <Show
        when={groupedEntities().length > 0}
        fallback={
          <div class="rounded-xl border border-dashed border-slate-800 p-8 text-center text-xs text-slate-400">
            No entities available in this area yet.
          </div>
        }
      >
        <div class="space-y-6">
          <For each={groupedEntities()}>
            {([areaKey, list]) => (
              <section class="space-y-3">
                <Show when={!fixedAreaId()}>
                  <h3 class="text-xs font-semibold uppercase tracking-wider text-slate-400">
                    {areaKey === "__unassigned__"
                      ? "Unassigned"
                      : areaNameById().get(areaKey) ?? areaKey}
                  </h3>
                </Show>
                <div class="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
                  <For each={list}>
                    {(entity) => <EntityCard entity={entity} />}
                  </For>
                </div>
              </section>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
};
