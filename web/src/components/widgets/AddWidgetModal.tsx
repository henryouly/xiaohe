import {
  createEffect,
  createMemo,
  createSignal,
  For,
  Show,
  type Component,
} from "solid-js";
import type {
  DashboardWidget,
  WidgetAccent,
  WidgetSize,
  WidgetType,
} from "../../lib/api";
import { useXiaohe } from "../../lib/store";
import { generateWidgetId, WIDGET_CATALOG } from "../../lib/widgets";

const ACCENTS: WidgetAccent[] = ["emerald", "sky", "amber", "rose", "violet"];
const DOMAINS = [
  "",
  "sensor",
  "binary_sensor",
  "switch",
  "light",
  "climate",
  "lock",
  "cover",
  "media_player",
  "button",
  "custom",
];
const WINDOW_OPTIONS = [1, 6, 24, 168];

export const AddWidgetModal: Component<{
  open: boolean;
  editingWidget: DashboardWidget | null;
  onClose: () => void;
  onSave: (widget: DashboardWidget) => void;
}> = (props) => {
  const store = useXiaohe();

  const [selectedType, setSelectedType] = createSignal<WidgetType>("metric");
  const [title, setTitle] = createSignal("");
  const [size, setSize] = createSignal<WidgetSize>("sm");
  const [entityId, setEntityId] = createSignal("");
  const [areaId, setAreaId] = createSignal("");
  const [domain, setDomain] = createSignal("");
  const [accent, setAccent] = createSignal<WidgetAccent>("emerald");
  const [windowHours, setWindowHours] = createSignal<number>(24);
  const [warnAbove, setWarnAbove] = createSignal<string>("");
  const [critAbove, setCritAbove] = createSignal<string>("");

  const allEntities = createMemo(() =>
    Object.values(store.entities).sort((a, b) => a.name.localeCompare(b.name)),
  );
  const numericSensors = createMemo(() =>
    allEntities().filter(
      (e) => e.state.domain === "sensor" && typeof e.state.value === "number",
    ),
  );

  // Sync form fields whenever modal opens or editingWidget changes
  createEffect(() => {
    if (!props.open) return;
    const existing = props.editingWidget;
    if (existing) {
      setSelectedType(existing.type);
      setTitle(existing.title ?? "");
      setSize(existing.size);
      setEntityId(
        "entity_id" in existing ? existing.entity_id : "",
      );
      setAreaId("area_id" in existing ? (existing.area_id ?? "") : "");
      setDomain("domain" in existing ? (existing.domain ?? "") : "");
      setAccent(
        "accent" in existing && existing.accent ? existing.accent : "emerald",
      );
      setWindowHours(
        existing.type === "sparkline" ? (existing.window_hours ?? 24) : 24,
      );
      setWarnAbove(
        existing.type === "metric" && existing.warn_above !== undefined
          ? String(existing.warn_above)
          : "",
      );
      setCritAbove(
        existing.type === "metric" && existing.crit_above !== undefined
          ? String(existing.crit_above)
          : "",
      );
    } else {
      setSelectedType("metric");
      setTitle("");
      setSize("sm");
      const firstSensor = numericSensors()[0]?.entity_id ?? allEntities()[0]?.entity_id ?? "";
      setEntityId(firstSensor);
      setAreaId("");
      setDomain("");
      setAccent("emerald");
      setWindowHours(24);
      setWarnAbove("");
      setCritAbove("");
    }
  });

  const handleSelectType = (type: WidgetType) => {
    setSelectedType(type);
    const meta = WIDGET_CATALOG.find((c) => c.type === type);
    if (meta && !props.editingWidget) {
      setSize(meta.defaultSize);
    }
    if (type === "metric" || type === "sparkline") {
      if (!numericSensors().some((e) => e.entity_id === entityId())) {
        setEntityId(numericSensors()[0]?.entity_id ?? allEntities()[0]?.entity_id ?? "");
      }
    } else if (type === "entity_card") {
      if (!allEntities().some((e) => e.entity_id === entityId())) {
        setEntityId(allEntities()[0]?.entity_id ?? "");
      }
    }
  };

  const needsEntity = () => {
    const t = selectedType();
    return t === "metric" || t === "sparkline" || t === "entity_card";
  };

  const handleSubmit = (e: Event) => {
    e.preventDefault();
    const id = props.editingWidget?.id ?? generateWidgetId(selectedType());
    const cleanTitle = title().trim() || undefined;
    const base = { id, title: cleanTitle, size: size() };
    const t = selectedType();

    let widget: DashboardWidget;
    switch (t) {
      case "metric": {
        const wNum = warnAbove().trim() === "" ? undefined : Number(warnAbove());
        const cNum = critAbove().trim() === "" ? undefined : Number(critAbove());
        widget = {
          ...base,
          type: "metric",
          entity_id: entityId(),
          accent: accent(),
          warn_above: Number.isFinite(wNum) ? wNum : undefined,
          crit_above: Number.isFinite(cNum) ? cNum : undefined,
        };
        break;
      }
      case "sparkline":
        widget = {
          ...base,
          type: "sparkline",
          entity_id: entityId(),
          window_hours: windowHours(),
          accent: accent(),
        };
        break;
      case "entity_card":
        widget = {
          ...base,
          type: "entity_card",
          entity_id: entityId(),
        };
        break;
      case "weather":
        widget = { ...base, type: "weather" };
        break;
      case "entity_list":
        widget = {
          ...base,
          type: "entity_list",
          area_id: areaId() || undefined,
          domain: domain() || undefined,
        };
        break;
      case "area_group":
        widget = {
          ...base,
          type: "area_group",
          area_id: areaId() || undefined,
        };
        break;
      case "system_status":
        widget = { ...base, type: "system_status" };
        break;
    }

    props.onSave(widget);
  };

  return (
    <Show when={props.open}>
      <div class="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/80 p-4 backdrop-blur-sm">
        <div class="w-full max-w-2xl max-h-[90vh] overflow-y-auto rounded-2xl border border-slate-800 bg-slate-900 p-6 shadow-2xl">
          <div class="flex items-center justify-between gap-3 border-b border-slate-800 pb-4">
            <div>
              <h2 class="text-lg font-semibold text-white">
                {props.editingWidget ? "Edit Widget" : "Add Widget to Dashboard"}
              </h2>
              <p class="text-xs text-slate-400">
                Choose a visualization and bind it to any live Xiaohe internal state.
              </p>
            </div>
            <button
              type="button"
              onClick={props.onClose}
              class="rounded-lg border border-slate-700 px-2.5 py-1 text-xs text-slate-300 hover:bg-slate-800"
            >
              Close
            </button>
          </div>

          <form onSubmit={handleSubmit} class="mt-5 space-y-5">
            {/* Widget type selector (locked when editing an existing widget) */}
            <Show when={!props.editingWidget}>
              <div>
                <label class="mb-2 block text-xs font-medium uppercase tracking-wider text-slate-400">
                  1. Widget Type
                </label>
                <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
                  <For each={WIDGET_CATALOG}>
                    {(entry) => (
                      <button
                        type="button"
                        onClick={() => handleSelectType(entry.type)}
                        class={`rounded-xl border p-3 text-left transition ${
                          selectedType() === entry.type
                            ? "border-emerald-500/60 bg-emerald-500/10 ring-1 ring-emerald-500/30"
                            : "border-slate-800 bg-slate-950/60 hover:border-slate-700"
                        }`}
                      >
                        <div class="flex items-center justify-between gap-2">
                          <span class="text-sm font-semibold text-white">
                            {entry.label}
                          </span>
                          <span class="rounded bg-slate-800 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-slate-400">
                            {entry.category}
                          </span>
                        </div>
                        <p class="mt-1 text-xs text-slate-400 leading-relaxed">
                          {entry.summary}
                        </p>
                      </button>
                    )}
                  </For>
                </div>
              </div>
            </Show>

            {/* Configuration fields */}
            <div class="space-y-4 rounded-xl border border-slate-800 bg-slate-950/50 p-4">
              <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
                <div>
                  <label class="mb-1 block text-xs text-slate-400">
                    Custom Title (optional)
                  </label>
                  <input
                    type="text"
                    value={title()}
                    onInput={(e) => setTitle(e.currentTarget.value)}
                    placeholder="Auto from entity / widget"
                    class="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-white"
                  />
                </div>

                <div>
                  <label class="mb-1 block text-xs text-slate-400">
                    Width on Desktop
                  </label>
                  <div class="grid grid-cols-3 gap-1.5">
                    {(["sm", "md", "lg"] as WidgetSize[]).map((s) => (
                      <button
                        type="button"
                        onClick={() => setSize(s)}
                        class={`rounded-lg border py-2 text-xs font-medium uppercase transition ${
                          size() === s
                            ? "border-emerald-500 bg-emerald-500/15 text-emerald-300"
                            : "border-slate-700 bg-slate-900 text-slate-300 hover:bg-slate-800"
                        }`}
                      >
                        {s === "sm" ? "Small (1/3)" : s === "md" ? "Medium (1/2)" : "Full"}
                      </button>
                    ))}
                  </div>
                </div>
              </div>

              {/* Entity picker for metric / sparkline / entity_card */}
              <Show when={needsEntity()}>
                <div>
                  <label class="mb-1 block text-xs text-slate-400">
                    Target Entity
                  </label>
                  <select
                    required
                    value={entityId()}
                    onChange={(e) => setEntityId(e.currentTarget.value)}
                    class="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-white"
                  >
                    <For
                      each={
                        selectedType() === "sparkline"
                          ? numericSensors().length > 0
                            ? numericSensors()
                            : allEntities()
                          : allEntities()
                      }
                    >
                      {(ent) => (
                        <option value={ent.entity_id}>
                          {ent.name} ({ent.entity_id})
                        </option>
                      )}
                    </For>
                  </select>
                </div>
              </Show>

              {/* Metric accent & thresholds */}
              <Show when={selectedType() === "metric"}>
                <div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
                  <div>
                    <label class="mb-1 block text-xs text-slate-400">
                      Accent Color
                    </label>
                    <select
                      value={accent()}
                      onChange={(e) =>
                        setAccent(e.currentTarget.value as WidgetAccent)
                      }
                      class="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm capitalize text-white"
                    >
                      <For each={ACCENTS}>
                        {(a) => <option value={a}>{a}</option>}
                      </For>
                    </select>
                  </div>
                  <div>
                    <label class="mb-1 block text-xs text-slate-400">
                      Warn Above (optional)
                    </label>
                    <input
                      type="number"
                      step="any"
                      value={warnAbove()}
                      onInput={(e) => setWarnAbove(e.currentTarget.value)}
                      placeholder="e.g. 35"
                      class="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-white"
                    />
                  </div>
                  <div>
                    <label class="mb-1 block text-xs text-slate-400">
                      Critical Above (optional)
                    </label>
                    <input
                      type="number"
                      step="any"
                      value={critAbove()}
                      onInput={(e) => setCritAbove(e.currentTarget.value)}
                      placeholder="e.g. 75"
                      class="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-white"
                    />
                  </div>
                </div>
              </Show>

              {/* Sparkline window & accent */}
              <Show when={selectedType() === "sparkline"}>
                <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
                  <div>
                    <label class="mb-1 block text-xs text-slate-400">
                      Time Window
                    </label>
                    <select
                      value={windowHours()}
                      onChange={(e) =>
                        setWindowHours(Number(e.currentTarget.value))
                      }
                      class="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-white"
                    >
                      <For each={WINDOW_OPTIONS}>
                        {(h) => (
                          <option value={h}>
                            {h === 168 ? "Last 7 days (168h)" : `Last ${h} hour${h > 1 ? "s" : ""}`}
                          </option>
                        )}
                      </For>
                    </select>
                  </div>
                  <div>
                    <label class="mb-1 block text-xs text-slate-400">
                      Line Color
                    </label>
                    <select
                      value={accent()}
                      onChange={(e) =>
                        setAccent(e.currentTarget.value as WidgetAccent)
                      }
                      class="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm capitalize text-white"
                    >
                      <For each={ACCENTS}>
                        {(a) => <option value={a}>{a}</option>}
                      </For>
                    </select>
                  </div>
                </div>
              </Show>

              {/* Area / domain filters for entity_list & area_group */}
              <Show
                when={
                  selectedType() === "entity_list" ||
                  selectedType() === "area_group"
                }
              >
                <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
                  <div>
                    <label class="mb-1 block text-xs text-slate-400">
                      Area Filter
                    </label>
                    <select
                      value={areaId()}
                      onChange={(e) => setAreaId(e.currentTarget.value)}
                      class="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-white"
                    >
                      <option value="">All areas</option>
                      <For each={store.areas()}>
                        {(a) => <option value={a.area_id}>{a.name}</option>}
                      </For>
                    </select>
                  </div>

                  <Show when={selectedType() === "entity_list"}>
                    <div>
                      <label class="mb-1 block text-xs text-slate-400">
                        Domain Filter
                      </label>
                      <select
                        value={domain()}
                        onChange={(e) => setDomain(e.currentTarget.value)}
                        class="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-white"
                      >
                        <For each={DOMAINS}>
                          {(d) => (
                            <option value={d}>
                              {d === "" ? "All domains" : d}
                            </option>
                          )}
                        </For>
                      </select>
                    </div>
                  </Show>
                </div>
              </Show>
            </div>

            <div class="flex items-center justify-end gap-2 pt-2">
              <button
                type="button"
                onClick={props.onClose}
                class="rounded-lg border border-slate-700 px-4 py-2 text-xs font-medium text-slate-300 hover:bg-slate-800"
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={needsEntity() && !entityId()}
                class="rounded-lg bg-emerald-500 px-4 py-2 text-xs font-semibold text-slate-950 hover:bg-emerald-400 disabled:opacity-50"
              >
                {props.editingWidget ? "Save Changes" : "Add to Dashboard"}
              </button>
            </div>
          </form>
        </div>
      </div>
    </Show>
  );
};
