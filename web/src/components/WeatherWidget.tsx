import { A } from "@solidjs/router";
import { createMemo, createSignal, Show, type Component } from "solid-js";
import { api, ApiError } from "../lib/api";
import { useXiaohe } from "../lib/store";
import { extractWeatherData, type WeatherIconKind } from "../lib/weather";

const WeatherIcon: Component<{ kind: WeatherIconKind }> = (props) => {
  switch (props.kind) {
    case "sun":
      return (
        <svg class="h-12 w-12 text-amber-300" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
          <circle cx="12" cy="12" r="4.5" fill="currentColor" fill-opacity="0.2" />
          <path stroke-linecap="round" d="M12 2.5v2.2M12 19.3v2.2M4.8 4.8l1.6 1.6M17.6 17.6l1.6 1.6M2.5 12h2.2M19.3 12h2.2M4.8 19.2l1.6-1.6M17.6 6.4l1.6-1.6" />
        </svg>
      );
    case "cloud-sun":
      return (
        <svg class="h-12 w-12 text-sky-300" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
          <circle cx="9.5" cy="9" r="3" class="text-amber-300" stroke="currentColor" fill="currentColor" fill-opacity="0.25" />
          <path stroke-linecap="round" d="M9.5 3.5v1.2M4.5 9h1.2M6 5.5l.9.9" class="text-amber-300" stroke="currentColor" />
          <path stroke-linejoin="round" fill="currentColor" fill-opacity="0.18" d="M8.5 18.5h8.2a3.3 3.3 0 0 0 .4-6.58A4.8 4.8 0 0 0 7.9 10.6a3.5 3.5 0 0 0 .6 7.9Z" />
        </svg>
      );
    case "rain":
      return (
        <svg class="h-12 w-12 text-sky-300" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
          <path stroke-linejoin="round" fill="currentColor" fill-opacity="0.18" d="M7 15h10a3.5 3.5 0 0 0 .4-6.98A5 5 0 0 0 7.6 6.8 3.8 3.8 0 0 0 7 15Z" />
          <path stroke-linecap="round" d="M9 18l-1 2.5M13 18l-1 2.5M17 18l-1 2.5" />
        </svg>
      );
    case "snow":
      return (
        <svg class="h-12 w-12 text-cyan-200" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
          <path stroke-linejoin="round" fill="currentColor" fill-opacity="0.18" d="M7 15h10a3.5 3.5 0 0 0 .4-6.98A5 5 0 0 0 7.6 6.8 3.8 3.8 0 0 0 7 15Z" />
          <circle cx="9" cy="19" r="1" fill="currentColor" />
          <circle cx="13" cy="20" r="1" fill="currentColor" />
          <circle cx="17" cy="19" r="1" fill="currentColor" />
        </svg>
      );
    case "storm":
      return (
        <svg class="h-12 w-12 text-amber-300" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
          <path stroke-linejoin="round" class="text-slate-300" stroke="currentColor" fill="currentColor" fill-opacity="0.18" d="M7 14.5h10a3.5 3.5 0 0 0 .4-6.98A5 5 0 0 0 7.6 6.3 3.8 3.8 0 0 0 7 14.5Z" />
          <path stroke-linecap="round" stroke-linejoin="round" d="M12.5 14.5l-2 4h3l-1.5 3.5" />
        </svg>
      );
    case "fog":
      return (
        <svg class="h-12 w-12 text-slate-300" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
          <path stroke-linecap="round" d="M4 10h16M6 14h12M4 18h14" />
        </svg>
      );
    default:
      return (
        <svg class="h-12 w-12 text-slate-300" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
          <path stroke-linejoin="round" fill="currentColor" fill-opacity="0.18" d="M7 17h10a3.8 3.8 0 0 0 .5-7.57A5.2 5.2 0 0 0 7.3 8.1 4 4 0 0 0 7 17Z" />
        </svg>
      );
  }
};

export const WeatherWidget: Component<{ showEmptyState?: boolean }> = (
  props,
) => {
  const store = useXiaohe();
  const [refreshing, setRefreshing] = createSignal(false);

  const weather = createMemo(() => extractWeatherData(store.entities));

  const handleRefresh = async () => {
    const w = weather();
    if (!w) return;
    setRefreshing(true);
    try {
      await api.commandEntity(w.refreshEntityId, "refresh", {});
      store.notify("success", "Weather updated");
    } catch (err) {
      store.notify(
        "error",
        err instanceof ApiError ? err.message : "Weather refresh failed",
      );
    } finally {
      setRefreshing(false);
    }
  };

  return (
    <Show
      when={weather()}
      fallback={
        <Show when={props.showEmptyState}>
          <div class="rounded-2xl border border-dashed border-sky-500/30 bg-slate-900/60 p-5 text-xs text-slate-400">
            Weather widget is waiting for{" "}
            <span class="font-mono text-sky-300">open-meteo-weather</span>{" "}
            sensor entities. Enable it in{" "}
            <A href="/skills" class="text-sky-300 underline">
              Skills
            </A>
            .
          </div>
        </Show>
      }
    >
      {(w) => (
        <section class="relative overflow-hidden rounded-2xl border border-sky-500/25 bg-gradient-to-br from-sky-950/60 via-slate-900/90 to-slate-900 p-5 shadow-lg">
          <div class="flex flex-wrap items-center justify-between gap-4">
            <div class="flex items-center gap-4">
              <div class="flex h-16 w-16 shrink-0 items-center justify-center rounded-2xl bg-sky-500/10 ring-1 ring-sky-400/25">
                <WeatherIcon kind={w().icon} />
              </div>
              <div>
                <div class="flex items-center gap-2">
                  <span class="text-xs font-semibold uppercase tracking-wider text-sky-300">
                    {w().location}
                  </span>
                  <Show when={w().provider}>
                    <span class="rounded bg-slate-800/90 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-slate-400">
                      {w().provider}
                    </span>
                  </Show>
                </div>
                <div class="mt-0.5 flex items-baseline gap-2.5">
                  <span class="text-3xl font-bold tracking-tight text-white">
                    {w().temperature !== null
                      ? `${w().temperature!.toFixed(1)}${w().temperatureUnit}`
                      : "—"}
                  </span>
                  <span class="text-sm font-medium text-slate-200">
                    {w().condition}
                  </span>
                </div>
                <Show when={w().apparentTemp !== null}>
                  <div class="text-xs text-slate-400">
                    Feels like {w().apparentTemp!.toFixed(1)}
                    {w().temperatureUnit}
                  </div>
                </Show>
              </div>
            </div>

            <div class="flex flex-wrap items-center gap-3">
              <Show when={w().humidity !== null}>
                <div class="rounded-xl border border-slate-800 bg-slate-950/60 px-3.5 py-2 text-center">
                  <div class="text-[10px] uppercase tracking-wider text-slate-400">
                    Humidity
                  </div>
                  <div class="text-sm font-semibold text-white">
                    {w().humidity}%
                  </div>
                </div>
              </Show>
              <Show when={w().windSpeed !== null}>
                <div class="rounded-xl border border-slate-800 bg-slate-950/60 px-3.5 py-2 text-center">
                  <div class="text-[10px] uppercase tracking-wider text-slate-400">
                    Wind
                  </div>
                  <div class="text-sm font-semibold text-white">
                    {w().windSpeed} {w().windUnit}
                  </div>
                </div>
              </Show>

              <div class="flex items-center gap-2">
                <button
                  type="button"
                  disabled={refreshing() || w().availability !== "online"}
                  onClick={() => void handleRefresh()}
                  class="rounded-lg border border-sky-500/35 bg-sky-500/15 px-3 py-2 text-xs font-medium text-sky-200 transition hover:bg-sky-500/25 disabled:opacity-50"
                >
                  {refreshing() ? "Updating…" : "Refresh"}
                </button>
                <A
                  href={`/history?entity=${encodeURIComponent(w().refreshEntityId)}`}
                  class="rounded-lg border border-slate-700 bg-slate-900/80 px-3 py-2 text-xs font-medium text-slate-200 hover:bg-slate-800"
                >
                  History →
                </A>
              </div>
            </div>
          </div>
        </section>
      )}
    </Show>
  );
};
