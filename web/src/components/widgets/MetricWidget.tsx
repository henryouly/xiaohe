import { A } from "@solidjs/router";
import { createMemo, Show, type Component } from "solid-js";
import type { MetricWidgetConfig, WidgetAccent } from "../../lib/api";
import { useXiaohe } from "../../lib/store";
import { formatEntityStateSummary } from "../../lib/widgets";

const ACCENT_RING: Record<WidgetAccent, string> = {
  emerald: "border-emerald-500/25 from-emerald-950/35",
  sky: "border-sky-500/25 from-sky-950/35",
  amber: "border-amber-500/25 from-amber-950/35",
  rose: "border-rose-500/25 from-rose-950/35",
  violet: "border-violet-500/25 from-violet-950/35",
};

const ACCENT_TEXT: Record<WidgetAccent, string> = {
  emerald: "text-emerald-300",
  sky: "text-sky-300",
  amber: "text-amber-300",
  rose: "text-rose-300",
  violet: "text-violet-300",
};

export const MetricWidget: Component<{ config: MetricWidgetConfig }> = (
  props,
) => {
  const store = useXiaohe();
  const entity = createMemo(() => store.entities[props.config.entity_id]);

  const numericValue = createMemo<number | null>(() => {
    const e = entity();
    if (!e) return null;
    if (e.state.domain === "sensor" && typeof e.state.value === "number") {
      return e.state.value;
    }
    if (
      e.state.domain === "climate" &&
      typeof e.state.current_temp_c === "number"
    ) {
      return e.state.current_temp_c;
    }
    return null;
  });

  const severity = createMemo<"normal" | "warn" | "crit">(() => {
    const v = numericValue();
    if (v === null) return "normal";
    if (
      props.config.crit_above !== undefined &&
      v >= props.config.crit_above
    ) {
      return "crit";
    }
    if (
      props.config.warn_above !== undefined &&
      v >= props.config.warn_above
    ) {
      return "warn";
    }
    return "normal";
  });

  const accent = (): WidgetAccent => {
    if (severity() === "crit") return "rose";
    if (severity() === "warn") return "amber";
    return props.config.accent ?? "emerald";
  };

  const displayParts = createMemo(() => {
    const e = entity();
    if (!e) return { value: "—", unit: "" };
    if (e.state.domain === "sensor") {
      const val =
        typeof e.state.value === "number"
          ? Number.isInteger(e.state.value)
            ? String(e.state.value)
            : e.state.value.toFixed(1)
          : String(e.state.value);
      return { value: val, unit: e.state.unit ?? "" };
    }
    return { value: formatEntityStateSummary(e), unit: "" };
  });

  return (
    <div
      class={`h-full rounded-2xl border bg-gradient-to-br via-slate-900/90 to-slate-900 p-5 ${ACCENT_RING[accent()]}`}
    >
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <div
            class={`truncate text-xs font-semibold uppercase tracking-wider ${ACCENT_TEXT[accent()]}`}
          >
            {props.config.title || entity()?.name || props.config.entity_id}
          </div>
          <div class="truncate text-[11px] font-mono text-slate-500">
            {props.config.entity_id}
          </div>
        </div>
        <div class="flex items-center gap-2 shrink-0">
          <Show when={severity() !== "normal"}>
            <span
              class={`rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase ${
                severity() === "crit"
                  ? "bg-rose-500/20 text-rose-300"
                  : "bg-amber-500/20 text-amber-300"
              }`}
            >
              {severity() === "crit" ? "High" : "Elevated"}
            </span>
          </Show>
          <span
            class={`inline-block h-2 w-2 rounded-full ${
              entity()?.availability === "online"
                ? "bg-emerald-400"
                : "bg-slate-600"
            }`}
          />
        </div>
      </div>

      <div class="mt-4 flex items-baseline justify-between gap-3">
        <Show
          when={entity()}
          fallback={
            <span class="text-sm text-slate-500">Entity unavailable</span>
          }
        >
          <div class="flex items-baseline gap-1.5">
            <span class="text-4xl font-bold tracking-tight text-white tabular-nums">
              {displayParts().value}
            </span>
            <Show when={displayParts().unit}>
              <span class="text-sm font-medium text-slate-400">
                {displayParts().unit}
              </span>
            </Show>
          </div>
        </Show>

        <A
          href={`/history?entity=${encodeURIComponent(props.config.entity_id)}`}
          class="rounded-lg border border-slate-700/80 bg-slate-900/80 px-2.5 py-1 text-[11px] font-medium text-slate-300 hover:bg-slate-800 hover:text-white"
        >
          History →
        </A>
      </div>
    </div>
  );
};
