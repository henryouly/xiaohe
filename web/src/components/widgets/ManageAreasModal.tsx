import { createSignal, For, Show, type Component } from "solid-js";
import { api, ApiError } from "../../lib/api";
import { useXiaohe } from "../../lib/store";

export const ManageAreasModal: Component<{
  open: boolean;
  onClose: () => void;
}> = (props) => {
  const store = useXiaohe();
  const [newAreaId, setNewAreaId] = createSignal("");
  const [newAreaName, setNewAreaName] = createSignal("");
  const [saving, setSaving] = createSignal(false);
  const [deletingId, setDeletingId] = createSignal<string | null>(null);

  const handleCreate = async (e: Event) => {
    e.preventDefault();
    setSaving(true);
    try {
      await api.createArea({
        area_id: newAreaId().trim(),
        name: newAreaName().trim(),
      });
      await store.refreshSnapshot();
      setNewAreaId("");
      setNewAreaName("");
      store.notify("success", "Area created");
    } catch (err) {
      store.notify(
        "error",
        err instanceof ApiError ? err.message : "Failed to create area",
      );
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (areaId: string) => {
    setDeletingId(areaId);
    try {
      await api.deleteArea(areaId);
      await store.refreshSnapshot();
      store.notify("success", `Deleted area '${areaId}'`);
    } catch (err) {
      store.notify(
        "error",
        err instanceof ApiError ? err.message : "Failed to delete area",
      );
    } finally {
      setDeletingId(null);
    }
  };

  return (
    <Show when={props.open}>
      <div class="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/80 p-4 backdrop-blur-sm">
        <div class="w-full max-w-lg rounded-2xl border border-slate-800 bg-slate-900 p-6 shadow-2xl">
          <div class="flex items-center justify-between gap-3 border-b border-slate-800 pb-4">
            <div>
              <h2 class="text-lg font-semibold text-white">Manage Areas</h2>
              <p class="text-xs text-slate-400">
                Organize devices and RBAC permissions by physical rooms or zones.
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

          <div class="mt-4 divide-y divide-slate-800/80 max-h-56 overflow-y-auto">
            <For each={store.areas()}>
              {(area) => (
                <div class="flex items-center justify-between gap-3 py-2.5">
                  <div>
                    <div class="text-sm font-medium text-white">{area.name}</div>
                    <div class="text-[11px] font-mono text-slate-500">
                      {area.area_id} · {area.entity_count} entities
                    </div>
                  </div>
                  <button
                    type="button"
                    disabled={deletingId() === area.area_id}
                    onClick={() => void handleDelete(area.area_id)}
                    class="rounded-md bg-rose-500/15 px-2.5 py-1 text-xs text-rose-300 hover:bg-rose-500/25 disabled:opacity-50"
                  >
                    Delete
                  </button>
                </div>
              )}
            </For>
          </div>

          <form
            onSubmit={handleCreate}
            class="mt-4 space-y-3 rounded-xl border border-slate-800 bg-slate-950/50 p-4"
          >
            <div class="text-xs font-semibold uppercase tracking-wider text-slate-400">
              Add New Area
            </div>
            <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <div>
                <label class="mb-1 block text-xs text-slate-400">Area ID</label>
                <input
                  type="text"
                  required
                  pattern="[a-z0-9_-]+"
                  placeholder="kitchen"
                  value={newAreaId()}
                  onInput={(e) => setNewAreaId(e.currentTarget.value)}
                  class="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-white"
                />
              </div>
              <div>
                <label class="mb-1 block text-xs text-slate-400">Name</label>
                <input
                  type="text"
                  required
                  placeholder="Kitchen"
                  value={newAreaName()}
                  onInput={(e) => setNewAreaName(e.currentTarget.value)}
                  class="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-white"
                />
              </div>
            </div>
            <div class="flex justify-end">
              <button
                type="submit"
                disabled={saving()}
                class="rounded-lg bg-emerald-500 px-4 py-2 text-xs font-semibold text-slate-950 hover:bg-emerald-400 disabled:opacity-50"
              >
                {saving() ? "Saving…" : "Create Area"}
              </button>
            </div>
          </form>
        </div>
      </div>
    </Show>
  );
};
