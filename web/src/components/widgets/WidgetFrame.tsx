import { Show, type ParentComponent } from "solid-js";
import type { WidgetSize } from "../../lib/api";

const SIZE_CLASS: Record<WidgetSize, string> = {
  sm: "md:col-span-3 lg:col-span-4",
  md: "md:col-span-6 lg:col-span-6",
  lg: "md:col-span-6 lg:col-span-12",
};

const SIZE_LABEL: Record<WidgetSize, string> = {
  sm: "S",
  md: "M",
  lg: "L",
};

export const WidgetFrame: ParentComponent<{
  size: WidgetSize;
  editing: boolean;
  canMovePrev: boolean;
  canMoveNext: boolean;
  onMovePrev: () => void;
  onMoveNext: () => void;
  onCycleSize: () => void;
  onEdit: () => void;
  onRemove: () => void;
}> = (props) => {
  return (
    <div
      class={`col-span-1 ${SIZE_CLASS[props.size]} relative transition ${
        props.editing
          ? "rounded-2xl ring-2 ring-emerald-500/40 ring-offset-2 ring-offset-slate-950"
          : ""
      }`}
    >
      <Show when={props.editing}>
        <div class="mb-2 flex items-center justify-between gap-2 rounded-xl border border-emerald-500/30 bg-slate-900/95 px-3 py-1.5 text-xs">
          <div class="flex items-center gap-1">
            <button
              type="button"
              disabled={!props.canMovePrev}
              onClick={props.onMovePrev}
              title="Move earlier"
              class="rounded-md bg-slate-800 px-2 py-0.5 text-slate-200 hover:bg-slate-700 disabled:opacity-35"
            >
              ←
            </button>
            <button
              type="button"
              disabled={!props.canMoveNext}
              onClick={props.onMoveNext}
              title="Move later"
              class="rounded-md bg-slate-800 px-2 py-0.5 text-slate-200 hover:bg-slate-700 disabled:opacity-35"
            >
              →
            </button>
            <button
              type="button"
              onClick={props.onCycleSize}
              title="Cycle widget width (S / M / L)"
              class="ml-1 rounded-md border border-slate-700 bg-slate-800 px-2 py-0.5 font-mono text-[11px] text-emerald-300 hover:bg-slate-700"
            >
              Width: {SIZE_LABEL[props.size]}
            </button>
          </div>
          <div class="flex items-center gap-1.5">
            <button
              type="button"
              onClick={props.onEdit}
              class="rounded-md bg-slate-800 px-2 py-0.5 text-slate-200 hover:bg-slate-700"
            >
              Edit
            </button>
            <button
              type="button"
              onClick={props.onRemove}
              class="rounded-md bg-rose-500/15 px-2 py-0.5 text-rose-300 hover:bg-rose-500/25"
            >
              Remove
            </button>
          </div>
        </div>
      </Show>
      {props.children}
    </div>
  );
};
