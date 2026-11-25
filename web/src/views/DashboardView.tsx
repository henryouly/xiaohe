import {
  createEffect,
  createMemo,
  createSignal,
  For,
  Show,
  type Component,
} from "solid-js";
import { EntityCard } from "../components/EntityCard";
import { WeatherWidget } from "../components/WeatherWidget";
import { AddWidgetModal } from "../components/widgets/AddWidgetModal";
import { AreaGroupWidget } from "../components/widgets/AreaGroupWidget";
import { EntityListWidget } from "../components/widgets/EntityListWidget";
import { ManageAreasModal } from "../components/widgets/ManageAreasModal";
import { MetricWidget } from "../components/widgets/MetricWidget";
import { SparklineWidget } from "../components/widgets/SparklineWidget";
import { SystemStatusWidget } from "../components/widgets/SystemStatusWidget";
import { WidgetFrame } from "../components/widgets/WidgetFrame";
import { api, ApiError, type DashboardWidget } from "../lib/api";
import { useXiaohe } from "../lib/store";
import {
  addWidget,
  cycleWidgetSize,
  moveWidget,
  removeWidget,
  updateWidget,
} from "../lib/widgets";

export const DashboardView: Component = () => {
  const store = useXiaohe();
  const [widgets, setWidgets] = createSignal<DashboardWidget[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [saving, setSaving] = createSignal(false);
  const [editMode, setEditMode] = createSignal(false);
  const [modalOpen, setModalOpen] = createSignal(false);
  const [editingWidget, setEditingWidget] = createSignal<DashboardWidget | null>(
    null,
  );
  const [areasModalOpen, setAreasModalOpen] = createSignal(false);

  createEffect(() => {
    let cancelled = false;
    setLoading(true);
    void api
      .getDashboard()
      .then((cfg) => {
        if (!cancelled) setWidgets(cfg.widgets);
      })
      .catch((err) => {
        if (!cancelled) {
          store.notify(
            "error",
            err instanceof ApiError ? err.message : "Failed to load dashboard",
          );
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  });

  const persistWidgets = async (next: DashboardWidget[], toast?: string) => {
    const prev = widgets();
    setWidgets(next);
    setSaving(true);
    try {
      const saved = await api.saveDashboard({ widgets: next });
      setWidgets(saved.widgets);
      if (toast) store.notify("success", toast);
    } catch (err) {
      setWidgets(prev);
      store.notify(
        "error",
        err instanceof ApiError ? err.message : "Failed to save dashboard",
      );
    } finally {
      setSaving(false);
    }
  };

  const handleSaveWidget = (widget: DashboardWidget) => {
    const isEdit = editingWidget() !== null;
    const next = isEdit
      ? updateWidget(widgets(), widget)
      : addWidget(widgets(), widget);
    setModalOpen(false);
    setEditingWidget(null);
    void persistWidgets(next, isEdit ? "Widget updated" : "Widget added");
  };

  const openAddModal = () => {
    setEditingWidget(null);
    setModalOpen(true);
  };

  const openEditModal = (widget: DashboardWidget) => {
    setEditingWidget(widget);
    setModalOpen(true);
  };

  const onlineCount = createMemo(
    () =>
      Object.values(store.entities).filter((e) => e.availability === "online")
        .length,
  );
  const totalEntities = createMemo(() => Object.keys(store.entities).length);

  const renderWidgetBody = (widget: DashboardWidget) => {
    switch (widget.type) {
      case "metric":
        return <MetricWidget config={widget} />;
      case "sparkline":
        return <SparklineWidget config={widget} />;
      case "entity_card": {
        const entity = () => store.entities[widget.entity_id];
        return (
          <Show
            when={entity()}
            fallback={
              <div class="h-full rounded-2xl border border-dashed border-slate-800 bg-slate-900/50 p-5 text-xs text-slate-500">
                Entity <span class="font-mono">{widget.entity_id}</span> is
                unavailable or hidden by area permissions.
              </div>
            }
          >
            {(ent) => (
              <EntityCard entity={ent()} titleOverride={widget.title} />
            )}
          </Show>
        );
      }
      case "weather":
        return <WeatherWidget showEmptyState={editMode()} />;
      case "entity_list":
        return <EntityListWidget config={widget} />;
      case "area_group":
        return <AreaGroupWidget config={widget} />;
      case "system_status":
        return <SystemStatusWidget config={widget} />;
    }
  };

  return (
    <div class="space-y-6">
      {/* Header */}
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 class="text-2xl font-semibold text-white">Dashboard</h1>
          <p class="text-sm text-slate-400">
            {widgets().length} widget{widgets().length === 1 ? "" : "s"} ·{" "}
            {onlineCount()}/{totalEntities()} entities online
            <Show when={saving()}>
              <span class="ml-2 text-xs text-emerald-400">Saving…</span>
            </Show>
          </p>
        </div>

        <Show when={store.isAdmin()}>
          <div class="flex flex-wrap items-center gap-2">
            <button
              type="button"
              onClick={openAddModal}
              class="rounded-lg bg-emerald-500 px-3.5 py-2 text-xs font-semibold text-slate-950 transition hover:bg-emerald-400"
            >
              + Add Widget
            </button>
            <button
              type="button"
              onClick={() => setEditMode((v) => !v)}
              class={`rounded-lg border px-3.5 py-2 text-xs font-medium transition ${
                editMode()
                  ? "border-emerald-500/60 bg-emerald-500/15 text-emerald-300"
                  : "border-slate-700 bg-slate-900 text-slate-200 hover:bg-slate-800"
              }`}
            >
              {editMode() ? "Done" : "Customize"}
            </button>
            <button
              type="button"
              onClick={() => setAreasModalOpen(true)}
              class="rounded-lg border border-slate-800 bg-slate-900/60 px-3 py-2 text-xs font-medium text-slate-400 transition hover:border-slate-700 hover:text-slate-200"
            >
              Areas
            </button>
          </div>
        </Show>
      </div>

      {/* Widget Grid */}
      <Show
        when={!loading()}
        fallback={
          <div class="rounded-2xl border border-slate-800/80 bg-slate-900/40 p-10 text-center text-sm text-slate-400">
            Loading dashboard layout…
          </div>
        }
      >
        <Show
          when={widgets().length > 0}
          fallback={
            <div class="rounded-2xl border border-dashed border-slate-800 p-12 text-center space-y-3">
              <div class="text-sm text-slate-300">
                Your dashboard has no widgets yet.
              </div>
              <Show when={store.isAdmin()}>
                <button
                  type="button"
                  onClick={openAddModal}
                  class="rounded-lg bg-emerald-500 px-4 py-2 text-xs font-semibold text-slate-950 hover:bg-emerald-400"
                >
                  + Add Your First Widget
                </button>
              </Show>
            </div>
          }
        >
          <div class="grid grid-cols-1 gap-5 md:grid-cols-6 lg:grid-cols-12">
            <For each={widgets()}>
              {(widget, idx) => (
                <WidgetFrame
                  size={widget.size}
                  editing={editMode() && store.isAdmin()}
                  canMovePrev={idx() > 0}
                  canMoveNext={idx() < widgets().length - 1}
                  onMovePrev={() =>
                    void persistWidgets(moveWidget(widgets(), widget.id, -1))
                  }
                  onMoveNext={() =>
                    void persistWidgets(moveWidget(widgets(), widget.id, 1))
                  }
                  onCycleSize={() =>
                    void persistWidgets(cycleWidgetSize(widgets(), widget.id))
                  }
                  onEdit={() => openEditModal(widget)}
                  onRemove={() =>
                    void persistWidgets(
                      removeWidget(widgets(), widget.id),
                      "Widget removed",
                    )
                  }
                >
                  {renderWidgetBody(widget)}
                </WidgetFrame>
              )}
            </For>
          </div>
        </Show>
      </Show>

      <AddWidgetModal
        open={modalOpen()}
        editingWidget={editingWidget()}
        onClose={() => {
          setModalOpen(false);
          setEditingWidget(null);
        }}
        onSave={handleSaveWidget}
      />

      <ManageAreasModal
        open={areasModalOpen()}
        onClose={() => setAreasModalOpen(false)}
      />
    </div>
  );
};
