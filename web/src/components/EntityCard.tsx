import { A } from "@solidjs/router";
import { createSignal, For, Show, type Component } from "solid-js";
import {
  api,
  ApiError,
  type ClimateMode,
  type EntityRecord,
} from "../lib/api";
import { useXiaohe } from "../lib/store";

const CLIMATE_MODES: ClimateMode[] = [
  "off",
  "heat",
  "cool",
  "auto",
  "dry",
  "fan_only",
];

export const EntityCard: Component<{
  entity: EntityRecord;
  titleOverride?: string;
}> = (props) => {
  const store = useXiaohe();
  const [busy, setBusy] = createSignal(false);
  const [confirmUnlock, setConfirmUnlock] = createSignal(false);

  const isOffline = () => props.entity.availability !== "online";
  const isGuestLockBlocked = () => {
    const u = store.user();
    return (
      props.entity.state.domain === "lock" &&
      u?.role === "guest" &&
      !u.allow_guest_locks
    );
  };

  const sendCommand = async (
    action: string,
    params: Record<string, unknown> = {},
  ) => {
    setBusy(true);
    try {
      await api.commandEntity(props.entity.entity_id, action, params);
      setConfirmUnlock(false);
    } catch (err) {
      const msg = err instanceof ApiError ? err.message : "Command failed";
      store.notify("error", `${props.entity.name}: ${msg}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      class={`h-full rounded-2xl border p-4 transition ${
        isOffline()
          ? "border-slate-800/70 bg-slate-900/40 opacity-70"
          : "border-slate-800 bg-slate-900/80 hover:border-slate-700"
      }`}
    >
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <div class="truncate text-sm font-semibold text-white">
            {props.titleOverride || props.entity.name}
          </div>
          <div class="truncate text-[11px] font-mono text-slate-500">
            {props.entity.entity_id}
          </div>
        </div>
        <div class="flex items-center gap-2 shrink-0">
          <span
            class={`inline-block h-2 w-2 rounded-full ${
              props.entity.availability === "online"
                ? "bg-emerald-400"
                : props.entity.availability === "offline"
                  ? "bg-rose-400"
                  : "bg-slate-500"
            }`}
            title={props.entity.availability}
          />
          <A
            href={`/history?entity=${encodeURIComponent(props.entity.entity_id)}`}
            class="rounded-md bg-slate-800/80 px-2 py-0.5 text-[11px] text-slate-300 hover:bg-slate-700 hover:text-white"
          >
            History
          </A>
        </div>
      </div>

      {/* Domain-specific controls */}
      <div class="mt-4">
        {/* LIGHT */}
        <Show when={props.entity.state.domain === "light"}>
          {(() => {
            const st = () =>
              props.entity.state as Extract<
                EntityRecord["state"],
                { domain: "light" }
              >;
            return (
              <div class="space-y-3">
                <div class="flex items-center justify-between">
                  <span class="text-xs text-slate-400">
                    {st().on ? "On" : "Off"}
                    {st().brightness !== undefined ? ` · ${st().brightness}%` : ""}
                  </span>
                  <button
                    type="button"
                    disabled={busy() || isOffline()}
                    onClick={() =>
                      void sendCommand(st().on ? "turn_off" : "turn_on")
                    }
                    class={`rounded-full px-3.5 py-1 text-xs font-semibold transition ${
                      st().on
                        ? "bg-amber-400 text-slate-950 hover:bg-amber-300"
                        : "bg-slate-800 text-slate-300 hover:bg-slate-700"
                    } disabled:opacity-50`}
                  >
                    {st().on ? "ON" : "OFF"}
                  </button>
                </div>
                <label class="block">
                  <div class="mb-1 flex justify-between text-[11px] text-slate-400">
                    <span>Brightness</span>
                    <span>{st().brightness ?? (st().on ? 100 : 0)}%</span>
                  </div>
                  <input
                    type="range"
                    min="0"
                    max="100"
                    disabled={busy() || isOffline()}
                    value={st().brightness ?? (st().on ? 100 : 0)}
                    onChange={(e) =>
                      void sendCommand("turn_on", {
                        brightness: Number(e.currentTarget.value),
                      })
                    }
                    class="w-full accent-amber-400"
                  />
                </label>
                <label class="block">
                  <div class="mb-1 flex justify-between text-[11px] text-slate-400">
                    <span>Color Temp</span>
                    <span>{st().color_temp_k ?? 4000} K</span>
                  </div>
                  <input
                    type="range"
                    min="2700"
                    max="6500"
                    step="100"
                    disabled={busy() || isOffline()}
                    value={st().color_temp_k ?? 4000}
                    onChange={(e) =>
                      void sendCommand("turn_on", {
                        color_temp_k: Number(e.currentTarget.value),
                      })
                    }
                    class="w-full accent-sky-400"
                  />
                </label>
              </div>
            );
          })()}
        </Show>

        {/* SWITCH */}
        <Show when={props.entity.state.domain === "switch"}>
          {(() => {
            const st = () =>
              props.entity.state as Extract<
                EntityRecord["state"],
                { domain: "switch" }
              >;
            return (
              <div class="flex items-center justify-between">
                <span class="text-sm text-slate-300">
                  {st().on ? "Active" : "Off"}
                </span>
                <button
                  type="button"
                  disabled={busy() || isOffline()}
                  onClick={() =>
                    void sendCommand(st().on ? "turn_off" : "turn_on")
                  }
                  class={`rounded-full px-4 py-1.5 text-xs font-semibold transition ${
                    st().on
                      ? "bg-emerald-400 text-slate-950 hover:bg-emerald-300"
                      : "bg-slate-800 text-slate-300 hover:bg-slate-700"
                  } disabled:opacity-50`}
                >
                  {st().on ? "ON" : "OFF"}
                </button>
              </div>
            );
          })()}
        </Show>

        {/* CLIMATE */}
        <Show when={props.entity.state.domain === "climate"}>
          {(() => {
            const st = () =>
              props.entity.state as Extract<
                EntityRecord["state"],
                { domain: "climate" }
              >;
            const target = () => st().target_temp_c ?? 22;
            return (
              <div class="space-y-3">
                <div class="flex items-center justify-between">
                  <div>
                    <div class="text-2xl font-semibold text-white">
                      {target().toFixed(1)}°C
                    </div>
                    <Show when={st().current_temp_c !== undefined}>
                      <div class="text-xs text-slate-400">
                        Current: {st().current_temp_c?.toFixed(1)}°C
                      </div>
                    </Show>
                  </div>
                  <div class="flex items-center gap-1.5">
                    <button
                      type="button"
                      disabled={busy() || isOffline()}
                      onClick={() =>
                        void sendCommand("set_temperature", {
                          target_temp_c: Number((target() - 0.5).toFixed(1)),
                        })
                      }
                      class="h-8 w-8 rounded-lg border border-slate-700 bg-slate-800 text-sm font-bold text-white hover:bg-slate-700 disabled:opacity-50"
                    >
                      −
                    </button>
                    <button
                      type="button"
                      disabled={busy() || isOffline()}
                      onClick={() =>
                        void sendCommand("set_temperature", {
                          target_temp_c: Number((target() + 0.5).toFixed(1)),
                        })
                      }
                      class="h-8 w-8 rounded-lg border border-slate-700 bg-slate-800 text-sm font-bold text-white hover:bg-slate-700 disabled:opacity-50"
                    >
                      +
                    </button>
                  </div>
                </div>
                <div class="flex flex-wrap gap-1.5">
                  <For each={CLIMATE_MODES}>
                    {(mode) => (
                      <button
                        type="button"
                        disabled={busy() || isOffline()}
                        onClick={() => void sendCommand("set_mode", { mode })}
                        class={`rounded-md px-2 py-1 text-[11px] capitalize transition ${
                          st().mode === mode
                            ? "bg-sky-500 text-slate-950 font-semibold"
                            : "bg-slate-800 text-slate-300 hover:bg-slate-700"
                        } disabled:opacity-50`}
                      >
                        {mode.replace("_", " ")}
                      </button>
                    )}
                  </For>
                </div>
              </div>
            );
          })()}
        </Show>

        {/* LOCK */}
        <Show when={props.entity.state.domain === "lock"}>
          {(() => {
            const st = () =>
              props.entity.state as Extract<
                EntityRecord["state"],
                { domain: "lock" }
              >;
            return (
              <div class="space-y-2">
                <div class="flex items-center justify-between">
                  <span
                    class={`rounded-full px-2.5 py-0.5 text-xs font-medium capitalize ${
                      st().state === "locked"
                        ? "bg-emerald-500/15 text-emerald-300"
                        : st().state === "jammed"
                          ? "bg-rose-500/20 text-rose-300"
                          : "bg-amber-500/20 text-amber-300"
                    }`}
                  >
                    {st().state}
                  </span>

                  <Show
                    when={!isGuestLockBlocked()}
                    fallback={
                      <span class="text-xs text-slate-500">
                        Restricted for Guest
                      </span>
                    }
                  >
                    <Show
                      when={st().state === "locked"}
                      fallback={
                        <button
                          type="button"
                          disabled={busy() || isOffline()}
                          onClick={() => void sendCommand("lock")}
                          class="rounded-lg bg-emerald-500 px-3 py-1.5 text-xs font-semibold text-slate-950 hover:bg-emerald-400 disabled:opacity-50"
                        >
                          Lock
                        </button>
                      }
                    >
                      <Show
                        when={confirmUnlock()}
                        fallback={
                          <button
                            type="button"
                            disabled={busy() || isOffline()}
                            onClick={() => setConfirmUnlock(true)}
                            class="rounded-lg border border-amber-500/50 bg-amber-500/15 px-3 py-1.5 text-xs font-semibold text-amber-200 hover:bg-amber-500/25 disabled:opacity-50"
                          >
                            Unlock…
                          </button>
                        }
                      >
                        <div class="flex items-center gap-1.5">
                          <button
                            type="button"
                            disabled={busy()}
                            onClick={() => void sendCommand("unlock")}
                            class="rounded-lg bg-rose-500 px-2.5 py-1 text-xs font-semibold text-white hover:bg-rose-400"
                          >
                            Confirm
                          </button>
                          <button
                            type="button"
                            onClick={() => setConfirmUnlock(false)}
                            class="rounded-lg border border-slate-700 px-2 py-1 text-xs text-slate-300"
                          >
                            Cancel
                          </button>
                        </div>
                      </Show>
                    </Show>
                  </Show>
                </div>
              </div>
            );
          })()}
        </Show>

        {/* COVER */}
        <Show when={props.entity.state.domain === "cover"}>
          {(() => {
            const st = () =>
              props.entity.state as Extract<
                EntityRecord["state"],
                { domain: "cover" }
              >;
            return (
              <div class="space-y-2.5">
                <div class="flex items-center justify-between text-xs text-slate-400">
                  <span class="capitalize">{st().state}</span>
                  <Show when={st().position !== undefined}>
                    <span>{st().position}%</span>
                  </Show>
                </div>
                <div class="grid grid-cols-3 gap-1.5">
                  <button
                    type="button"
                    disabled={busy() || isOffline()}
                    onClick={() => void sendCommand("open")}
                    class="rounded-lg bg-slate-800 py-1.5 text-xs text-slate-200 hover:bg-slate-700 disabled:opacity-50"
                  >
                    Open
                  </button>
                  <button
                    type="button"
                    disabled={busy() || isOffline()}
                    onClick={() => void sendCommand("stop")}
                    class="rounded-lg bg-slate-800 py-1.5 text-xs text-slate-200 hover:bg-slate-700 disabled:opacity-50"
                  >
                    Stop
                  </button>
                  <button
                    type="button"
                    disabled={busy() || isOffline()}
                    onClick={() => void sendCommand("close")}
                    class="rounded-lg bg-slate-800 py-1.5 text-xs text-slate-200 hover:bg-slate-700 disabled:opacity-50"
                  >
                    Close
                  </button>
                </div>
              </div>
            );
          })()}
        </Show>

        {/* SENSOR */}
        <Show when={props.entity.state.domain === "sensor"}>
          {(() => {
            const st = () =>
              props.entity.state as Extract<
                EntityRecord["state"],
                { domain: "sensor" }
              >;
            return (
              <div class="flex items-baseline gap-1.5">
                <span class="text-2xl font-semibold tracking-tight text-white">
                  {String(st().value)}
                </span>
                <Show when={st().unit}>
                  <span class="text-xs font-medium text-slate-400">
                    {st().unit}
                  </span>
                </Show>
              </div>
            );
          })()}
        </Show>

        {/* BINARY SENSOR */}
        <Show when={props.entity.state.domain === "binary_sensor"}>
          {(() => {
            const st = () =>
              props.entity.state as Extract<
                EntityRecord["state"],
                { domain: "binary_sensor" }
              >;
            return (
              <span
                class={`inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium ${
                  st().active
                    ? "bg-amber-500/20 text-amber-300"
                    : "bg-slate-800 text-slate-400"
                }`}
              >
                {st().active ? "Detected / Active" : "Clear"}
              </span>
            );
          })()}
        </Show>

        {/* MEDIA PLAYER */}
        <Show when={props.entity.state.domain === "media_player"}>
          {(() => {
            const st = () =>
              props.entity.state as Extract<
                EntityRecord["state"],
                { domain: "media_player" }
              >;
            return (
              <div class="flex items-center justify-between gap-2">
                <div class="truncate text-xs text-slate-300">
                  {st().title || st().state}
                </div>
                <button
                  type="button"
                  disabled={busy() || isOffline()}
                  onClick={() =>
                    void sendCommand(
                      st().state === "playing" ? "pause" : "play",
                    )
                  }
                  class="rounded-lg bg-slate-800 px-3 py-1 text-xs font-medium text-white hover:bg-slate-700 disabled:opacity-50"
                >
                  {st().state === "playing" ? "Pause" : "Play"}
                </button>
              </div>
            );
          })()}
        </Show>

        {/* BUTTON */}
        <Show when={props.entity.state.domain === "button"}>
          <button
            type="button"
            disabled={busy() || isOffline()}
            onClick={() => void sendCommand("press")}
            class="w-full rounded-lg bg-slate-800 py-1.5 text-xs font-medium text-slate-200 hover:bg-slate-700 disabled:opacity-50"
          >
            Press
          </button>
        </Show>

        {/* CUSTOM */}
        <Show when={props.entity.state.domain === "custom"}>
          {(() => {
            const st = () =>
              props.entity.state as Extract<
                EntityRecord["state"],
                { domain: "custom" }
              >;
            return (
              <div class="text-sm font-medium text-slate-200">{st().state}</div>
            );
          })()}
        </Show>
      </div>
    </div>
  );
};
