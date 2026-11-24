import { useSearchParams } from "@solidjs/router";
import {
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  Show,
  type Component,
} from "solid-js";
import uPlot from "uplot";
import {
  api,
  ApiError,
  type EntityHistorySeries,
  type NumericPoint,
} from "../lib/api";
import { useXiaohe } from "../lib/store";

interface RangePreset {
  id: string;
  label: string;
  durationMs: number;
  bucketSecs: number;
}

const RANGE_PRESETS: RangePreset[] = [
  { id: "1h", label: "1h", durationMs: 3600_000, bucketSecs: 60 },
  { id: "6h", label: "6h", durationMs: 6 * 3600_000, bucketSecs: 300 },
  { id: "24h", label: "24h", durationMs: 24 * 3600_000, bucketSecs: 900 },
  { id: "7d", label: "7d", durationMs: 7 * 24 * 3600_000, bucketSecs: 3600 },
];

const NumericChart: Component<{ points: NumericPoint[]; unit?: string }> = (
  props,
) => {
  let container: HTMLDivElement | undefined;

  createEffect(() => {
    const pts = props.points;
    if (!container || pts.length === 0) return;

    const xs = pts.map((p) => p.ts / 1000);
    const avgs = pts.map((p) => p.avg);
    const mins = pts.map((p) => p.min);
    const maxs = pts.map((p) => p.max);

    const width = Math.max(container.clientWidth, 300);
    const unitLabel = props.unit ? ` (${props.unit})` : "";

    const opts: uPlot.Options = {
      width,
      height: 300,
      scales: { x: { time: true } },
      axes: [
        { stroke: "#94a3b8", grid: { stroke: "#1e293b" } },
        { stroke: "#94a3b8", grid: { stroke: "#1e293b" }, label: unitLabel },
      ],
      series: [
        {},
        { label: `Avg${unitLabel}`, stroke: "#34d399", width: 2 },
        { label: `Min${unitLabel}`, stroke: "#38bdf8", width: 1, dash: [4, 4] },
        { label: `Max${unitLabel}`, stroke: "#fbbf24", width: 1, dash: [4, 4] },
      ],
    };

    container.innerHTML = "";
    const plot = new uPlot(opts, [xs, avgs, mins, maxs], container);

    const ro = new ResizeObserver(() => {
      if (container) {
        plot.setSize({ width: Math.max(container.clientWidth, 300), height: 300 });
      }
    });
    ro.observe(container);

    onCleanup(() => {
      ro.disconnect();
      plot.destroy();
    });
  });

  return <div ref={container} class="w-full overflow-x-auto" />;
};

export const HistoryView: Component = () => {
  const store = useXiaohe();
  const [searchParams, setSearchParams] = useSearchParams();
  const [rangeId, setRangeId] = createSignal("24h");
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [series, setSeries] = createSignal<EntityHistorySeries | null>(null);

  const entityList = createMemo(() =>
    Object.values(store.entities).sort((a, b) =>
      a.entity_id.localeCompare(b.entity_id),
    ),
  );

  const selectedEntityId = createMemo(() => {
    const param = searchParams.entity;
    const fromParam = Array.isArray(param) ? param[0] : param;
    if (fromParam) return fromParam;
    return entityList()[0]?.entity_id ?? "";
  });

  const activePreset = createMemo(
    () => RANGE_PRESETS.find((r) => r.id === rangeId()) ?? RANGE_PRESETS[2],
  );

  createEffect(() => {
    const entityId = selectedEntityId();
    const preset = activePreset();
    if (!entityId) {
      setSeries(null);
      return;
    }

    let cancelled = false;
    setLoading(true);
    setError(null);

    const to = Date.now();
    const from = to - preset.durationMs;

    void api
      .entityHistory(entityId, {
        from,
        to,
        bucket_secs: preset.bucketSecs,
      })
      .then((res) => {
        if (!cancelled) setSeries(res);
      })
      .catch((err) => {
        if (!cancelled) {
          setError(err instanceof ApiError ? err.message : "Failed to load history");
          setSeries(null);
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    onCleanup(() => {
      cancelled = true;
    });
  });

  return (
    <div class="space-y-6">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 class="text-2xl font-semibold text-white">History & Timeline</h1>
          <p class="text-sm text-slate-400">
            Downsampled numeric telemetry (uPlot) and discrete state transitions
          </p>
        </div>

        <div class="flex flex-wrap items-center gap-2">
          <select
            value={selectedEntityId()}
            onChange={(e) => setSearchParams({ entity: e.currentTarget.value })}
            class="rounded-lg border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-white"
          >
            <For each={entityList()}>
              {(ent) => (
                <option value={ent.entity_id}>
                  {ent.name} ({ent.entity_id})
                </option>
              )}
            </For>
          </select>

          <div class="inline-flex rounded-lg border border-slate-800 bg-slate-900 p-0.5">
            <For each={RANGE_PRESETS}>
              {(preset) => (
                <button
                  type="button"
                  onClick={() => setRangeId(preset.id)}
                  class={`rounded-md px-3 py-1.5 text-xs font-medium transition ${
                    rangeId() === preset.id
                      ? "bg-emerald-500 text-slate-950"
                      : "text-slate-300 hover:text-white"
                  }`}
                >
                  {preset.label}
                </button>
              )}
            </For>
          </div>
        </div>
      </div>

      <Show when={error()}>
        <div class="rounded-xl border border-rose-500/40 bg-rose-500/10 p-4 text-sm text-rose-200">
          {error()}
        </div>
      </Show>

      <Show
        when={!loading()}
        fallback={
          <div class="rounded-2xl border border-slate-800 bg-slate-900/60 p-10 text-center text-sm text-slate-400">
            Loading history…
          </div>
        }
      >
        <Show
          when={series()}
          fallback={
            <div class="rounded-2xl border border-dashed border-slate-800 p-10 text-center text-sm text-slate-400">
              Select an entity to inspect its history.
            </div>
          }
        >
          {(data) => (
            <div class="rounded-2xl border border-slate-800 bg-slate-900/70 p-5">
              <Show when={data().kind === "numeric"}>
                {(() => {
                  const num = () =>
                    data() as Extract<EntityHistorySeries, { kind: "numeric" }>;
                  return (
                    <Show
                      when={num().points.length > 0}
                      fallback={
                        <p class="py-8 text-center text-sm text-slate-400">
                          No numeric telemetry recorded in this window.
                        </p>
                      }
                    >
                      <NumericChart points={num().points} unit={num().unit} />
                    </Show>
                  );
                })()}
              </Show>

              <Show when={data().kind === "discrete"}>
                {(() => {
                  const disc = () =>
                    data() as Extract<
                      EntityHistorySeries,
                      { kind: "discrete" }
                    >;
                  return (
                    <Show
                      when={disc().transitions.length > 0}
                      fallback={
                        <p class="py-8 text-center text-sm text-slate-400">
                          No state transitions recorded in this window.
                        </p>
                      }
                    >
                      <div class="overflow-x-auto">
                        <table class="w-full text-left text-sm">
                          <thead>
                            <tr class="border-b border-slate-800 text-xs uppercase text-slate-400">
                              <th class="py-2 pr-4">Timestamp</th>
                              <th class="py-2">State</th>
                            </tr>
                          </thead>
                          <tbody class="divide-y divide-slate-800/70">
                            <For each={[...disc().transitions].reverse()}>
                              {(tr) => (
                                <tr>
                                  <td class="py-2 pr-4 font-mono text-xs text-slate-400">
                                    {new Date(tr.ts).toLocaleString()}
                                  </td>
                                  <td class="py-2">
                                    <span class="rounded-full bg-slate-800 px-2.5 py-0.5 text-xs font-medium text-emerald-300">
                                      {tr.state}
                                    </span>
                                  </td>
                                </tr>
                              )}
                            </For>
                          </tbody>
                        </table>
                      </div>
                    </Show>
                  );
                })()}
              </Show>
            </div>
          )}
        </Show>
      </Show>
    </div>
  );
};
