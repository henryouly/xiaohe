import { A } from "@solidjs/router";
import {
  createEffect,
  createMemo,
  createSignal,
  Show,
  type Component,
} from "solid-js";
import {
  api,
  type NumericPoint,
  type SparklineWidgetConfig,
  type WidgetAccent,
} from "../../lib/api";
import { useXiaohe } from "../../lib/store";
import { buildSparklineGeometry } from "../../lib/widgets";

const STROKE_COLOR: Record<WidgetAccent, string> = {
  emerald: "#34d399",
  sky: "#38bdf8",
  amber: "#fbbf24",
  rose: "#fb7185",
  violet: "#a78bfa",
};

export const SparklineWidget: Component<{ config: SparklineWidgetConfig }> = (
  props,
) => {
  const store = useXiaohe();
  const [points, setPoints] = createSignal<NumericPoint[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [unit, setUnit] = createSignal<string>("");

  const entity = createMemo(() => store.entities[props.config.entity_id]);
  const windowHours = () => props.config.window_hours ?? 24;
  const accent = (): WidgetAccent => props.config.accent ?? "sky";

  createEffect(() => {
    const entityId = props.config.entity_id;
    const hours = windowHours();
    let cancelled = false;
    setLoading(true);
    const nowSec = Math.floor(Date.now() / 1000);
    const fromSec = nowSec - hours * 3600;
    const bucketSecs = Math.max(60, Math.floor((hours * 3600) / 80));
    void api
      .entityHistory(entityId, {
        from: fromSec,
        to: nowSec,
        bucket_secs: bucketSecs,
      })
      .then((res) => {
        if (cancelled) return;
        if (res.kind === "numeric") {
          setPoints(res.points);
          setUnit(res.unit ?? "");
        } else {
          setPoints([]);
        }
      })
      .catch(() => {
        if (!cancelled) setPoints([]);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  });

  // Append live SSE numeric reading onto the tail for real-time feedback
  const livePoints = createMemo<NumericPoint[]>(() => {
    const base = points();
    const e = entity();
    if (!e || e.state.domain !== "sensor" || typeof e.state.value !== "number") {
      return base;
    }
    const val = e.state.value;
    const ts = e.last_updated_at || Math.floor(Date.now() / 1000);
    if (base.length > 0 && base[base.length - 1].ts >= ts) {
      return base;
    }
    return [...base, { ts, avg: val, min: val, max: val }];
  });

  const geom = createMemo(() => buildSparklineGeometry(livePoints(), 320, 72));
  const resolvedUnit = () => {
    const e = entity();
    if (e && e.state.domain === "sensor" && e.state.unit) return e.state.unit;
    return unit();
  };

  const gradId = () => `spark-grad-${props.config.id}`;

  return (
    <div class="h-full rounded-2xl border border-slate-800 bg-slate-900/80 p-5 flex flex-col justify-between gap-3">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <div class="truncate text-sm font-semibold text-white">
            {props.config.title || entity()?.name || props.config.entity_id}
          </div>
          <div class="truncate text-[11px] font-mono text-slate-500">
            {props.config.entity_id} · last {windowHours()}h
          </div>
        </div>
        <div class="flex items-baseline gap-1 shrink-0">
          <Show when={geom()}>
            {(g) => (
              <>
                <span class="text-xl font-bold text-white tabular-nums">
                  {g().latest.toFixed(1)}
                </span>
                <span class="text-xs text-slate-400">{resolvedUnit()}</span>
              </>
            )}
          </Show>
        </div>
      </div>

      <Show
        when={!loading() && geom()}
        fallback={
          <div class="flex h-[72px] items-center justify-center rounded-xl border border-dashed border-slate-800 text-xs text-slate-500">
            {loading()
              ? "Loading telemetry…"
              : "No numeric history points yet"}
          </div>
        }
      >
        {(g) => (
          <div>
            <svg
              viewBox="0 0 320 72"
              class="h-[72px] w-full overflow-visible"
              preserveAspectRatio="none"
            >
              <defs>
                <linearGradient id={gradId()} x1="0" y1="0" x2="0" y2="1">
                  <stop
                    offset="0%"
                    stop-color={STROKE_COLOR[accent()]}
                    stop-opacity="0.32"
                  />
                  <stop
                    offset="100%"
                    stop-color={STROKE_COLOR[accent()]}
                    stop-opacity="0.02"
                  />
                </linearGradient>
              </defs>
              <path d={g().areaPath} fill={`url(#${gradId()})`} />
              <path
                d={g().linePath}
                fill="none"
                stroke={STROKE_COLOR[accent()]}
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              />
            </svg>
            <div class="mt-1 flex items-center justify-between text-[11px] text-slate-400">
              <span>
                Min {g().min.toFixed(1)}
                {resolvedUnit()} · Max {g().max.toFixed(1)}
                {resolvedUnit()}
              </span>
              <A
                href={`/history?entity=${encodeURIComponent(props.config.entity_id)}`}
                class="text-sky-300 hover:text-sky-200"
              >
                Full chart →
              </A>
            </div>
          </div>
        )}
      </Show>
    </div>
  );
};
