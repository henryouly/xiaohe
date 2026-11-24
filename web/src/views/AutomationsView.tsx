import {
  createSignal,
  For,
  onMount,
  Show,
  type Component,
} from "solid-js";
import {
  api,
  ApiError,
  type AutomationOutcome,
  type AutomationRunRecord,
  type ValidationErrorDetail,
} from "../lib/api";
import { useXiaohe } from "../lib/store";

const SAMPLE_AUTOMATION_TOML = `id = "my_automation"
name = "My Automation"
enabled = true

[[triggers]]
type = "webhook"
webhook_id = "my_hook"

[[actions]]
type = "entity.call"
entity_id = "switch.purifier_power"
action = "turn_on"
`;

export const AutomationsView: Component = () => {
  const store = useXiaohe();
  const [runs, setRuns] = createSignal<AutomationRunRecord[]>([]);
  const [runFilter, setRunFilter] = createSignal<string>("");
  const [skipConditions, setSkipConditions] = createSignal(true);

  // TOML editor state
  const [editorMode, setEditorMode] = createSignal<"closed" | "create" | "edit">(
    "closed",
  );
  const [editingId, setEditingId] = createSignal<string | null>(null);
  const [rawToml, setRawToml] = createSignal(SAMPLE_AUTOMATION_TOML);
  const [validationErrors, setValidationErrors] = createSignal<
    ValidationErrorDetail[]
  >([]);
  const [warnings, setWarnings] = createSignal<string[]>([]);
  const [saving, setSaving] = createSignal(false);

  const loadRuns = async (automationId?: string) => {
    try {
      const res = await api.listAutomationRuns({
        automation_id: automationId || undefined,
        limit: 50,
      });
      setRuns(res.runs);
    } catch {
      // Ignore transient errors
    }
  };

  onMount(() => {
    void store.refreshAdminData();
    void loadRuns();
  });

  const handleToggle = async (id: string, enabled: boolean) => {
    try {
      await api.toggleAutomation(id, enabled);
      await store.refreshAdminData();
      store.notify("success", `${id} ${enabled ? "enabled" : "disabled"}`);
    } catch (err) {
      store.notify(
        "error",
        err instanceof ApiError ? err.message : "Toggle failed",
      );
    }
  };

  const handleTrigger = async (id: string) => {
    try {
      const res = await api.triggerAutomation(id, skipConditions());
      store.notify("info", `Triggered ${id} (${res.run_id})`);
      window.setTimeout(() => void loadRuns(runFilter()), 500);
    } catch (err) {
      store.notify(
        "error",
        err instanceof ApiError ? err.message : "Trigger failed",
      );
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await api.deleteAutomation(id);
      await store.refreshAdminData();
      store.notify("success", `Deleted ${id}`);
    } catch (err) {
      store.notify(
        "error",
        err instanceof ApiError ? err.message : "Delete failed",
      );
    }
  };

  const handleReloadAll = async () => {
    try {
      const res = await api.reloadAutomations();
      await store.refreshAdminData();
      store.notify(
        res.errors.length > 0 ? "error" : "success",
        `Reloaded ${res.loaded} rules (${res.errors.length} errors)`,
      );
    } catch (err) {
      store.notify(
        "error",
        err instanceof ApiError ? err.message : "Reload failed",
      );
    }
  };

  const openEdit = async (id: string) => {
    try {
      const detail = await api.getAutomation(id);
      setEditingId(id);
      setRawToml(detail.raw_toml);
      setValidationErrors([]);
      setWarnings([]);
      setEditorMode("edit");
    } catch (err) {
      store.notify(
        "error",
        err instanceof ApiError ? err.message : "Failed to load automation",
      );
    }
  };

  const handleSaveToml = async (e: SubmitEvent) => {
    e.preventDefault();
    setValidationErrors([]);
    setWarnings([]);
    setSaving(true);
    try {
      const res =
        editorMode() === "create"
          ? await api.createAutomation(rawToml())
          : await api.updateAutomation(editingId()!, rawToml());
      setWarnings(res.warnings);
      await store.refreshAdminData();
      store.notify("success", `Saved automation '${res.id}'`);
      if (res.warnings.length === 0) {
        setEditorMode("closed");
      }
    } catch (err) {
      if (err instanceof ApiError) {
        if (err.details && err.details.length > 0) {
          setValidationErrors(err.details);
        } else {
          setValidationErrors([{ line: 1, message: err.message }]);
        }
      } else {
        setValidationErrors([{ line: 1, message: "Network error" }]);
      }
    } finally {
      setSaving(false);
    }
  };

  const outcomeBadge = (outcome: AutomationOutcome | null) => {
    switch (outcome) {
      case "succeeded":
        return "bg-emerald-500/15 text-emerald-300";
      case "failed":
        return "bg-rose-500/20 text-rose-300";
      case "conditions_not_met":
      case "skipped":
      case "skipped_single":
        return "bg-amber-500/15 text-amber-300";
      default:
        return "bg-slate-800 text-slate-400";
    }
  };

  return (
    <div class="space-y-6">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 class="text-2xl font-semibold text-white">
            Automations & Run Logs
          </h1>
          <p class="text-sm text-slate-400">
            Declarative TOML rules with server-side validation and SQLite audit
            trail
          </p>
        </div>
        <div class="flex flex-wrap items-center gap-2">
          <label class="flex items-center gap-1.5 text-xs text-slate-400 mr-2">
            <input
              type="checkbox"
              checked={skipConditions()}
              onChange={(e) => setSkipConditions(e.currentTarget.checked)}
              class="accent-emerald-500"
            />
            <span>Skip conditions on manual trigger</span>
          </label>
          <button
            type="button"
            onClick={() => void handleReloadAll()}
            class="rounded-lg border border-slate-700 bg-slate-900 px-3 py-2 text-xs font-medium text-slate-200 hover:bg-slate-800"
          >
            Reload All
          </button>
          <button
            type="button"
            onClick={() => {
              setEditingId(null);
              setRawToml(SAMPLE_AUTOMATION_TOML);
              setValidationErrors([]);
              setWarnings([]);
              setEditorMode("create");
            }}
            class="rounded-lg bg-emerald-500 px-3.5 py-2 text-xs font-semibold text-slate-950 hover:bg-emerald-400"
          >
            + New Automation
          </button>
        </div>
      </div>

      {/* TOML Editor */}
      <Show when={editorMode() !== "closed"}>
        <form
          onSubmit={handleSaveToml}
          class="rounded-2xl border border-slate-700 bg-slate-900 p-5 space-y-3"
        >
          <div class="flex items-center justify-between">
            <h2 class="text-base font-semibold text-white">
              {editorMode() === "create"
                ? "Create Automation (TOML)"
                : `Edit ${editingId()}`}
            </h2>
            <button
              type="button"
              onClick={() => setEditorMode("closed")}
              class="text-xs text-slate-400 hover:text-white"
            >
              Close
            </button>
          </div>

          <Show when={validationErrors().length > 0}>
            <div class="rounded-lg border border-rose-500/40 bg-rose-950/60 p-3 space-y-1 text-xs text-rose-200">
              <div class="font-semibold">Validation Failed (422)</div>
              <For each={validationErrors()}>
                {(err) => (
                  <div class="font-mono">
                    Line {err.line}: {err.message}
                  </div>
                )}
              </For>
            </div>
          </Show>

          <Show when={warnings().length > 0}>
            <div class="rounded-lg border border-amber-500/40 bg-amber-950/40 p-3 space-y-1 text-xs text-amber-200">
              <div class="font-semibold">Saved with warnings:</div>
              <For each={warnings()}>{(w) => <div>{w}</div>}</For>
            </div>
          </Show>

          <textarea
            rows={12}
            spellcheck={false}
            value={rawToml()}
            onInput={(e) => setRawToml(e.currentTarget.value)}
            class="w-full rounded-lg border border-slate-700 bg-slate-950 p-3 font-mono text-xs text-slate-100 focus:border-emerald-500 focus:outline-none"
          />

          <div class="flex justify-end gap-2">
            <button
              type="button"
              onClick={() => setEditorMode("closed")}
              class="rounded-lg border border-slate-700 px-3.5 py-2 text-xs text-slate-300"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={saving()}
              class="rounded-lg bg-emerald-500 px-4 py-2 text-xs font-semibold text-slate-950 hover:bg-emerald-400 disabled:opacity-50"
            >
              {saving() ? "Validating…" : "Validate & Save"}
            </button>
          </div>
        </form>
      </Show>

      {/* Rules list */}
      <div class="space-y-3">
        <Show
          when={store.automations().length > 0}
          fallback={
            <div class="rounded-2xl border border-dashed border-slate-800 p-8 text-center text-sm text-slate-400">
              No automations loaded yet.
            </div>
          }
        >
          <For each={store.automations()}>
            {(rule) => (
              <div class="flex flex-wrap items-center justify-between gap-4 rounded-2xl border border-slate-800 bg-slate-900/75 p-4">
                <div class="min-w-0">
                  <div class="flex items-center gap-2">
                    <span class="font-semibold text-white">{rule.name}</span>
                    <span class="rounded bg-slate-800 px-2 py-0.5 text-[10px] uppercase text-slate-400">
                      {rule.mode}
                    </span>
                    <Show when={rule.last_outcome}>
                      <span
                        class={`rounded-full px-2 py-0.5 text-[11px] font-medium ${outcomeBadge(rule.last_outcome)}`}
                      >
                        {rule.last_outcome}
                      </span>
                    </Show>
                  </div>
                  <div class="text-xs font-mono text-slate-500">
                    {rule.id} · {rule.file}
                    <Show when={rule.last_run_at}>
                      {" "}
                      · last run{" "}
                      {new Date(rule.last_run_at!).toLocaleTimeString()}
                    </Show>
                  </div>
                </div>

                <div class="flex flex-wrap items-center gap-2">
                  <button
                    type="button"
                    onClick={() => void handleToggle(rule.id, !rule.enabled)}
                    class={`rounded-full px-3 py-1 text-xs font-semibold transition ${
                      rule.enabled
                        ? "bg-emerald-500 text-slate-950"
                        : "bg-slate-800 text-slate-400"
                    }`}
                  >
                    {rule.enabled ? "Enabled" : "Disabled"}
                  </button>
                  <button
                    type="button"
                    onClick={() => void handleTrigger(rule.id)}
                    class="rounded-lg bg-sky-500/15 px-3 py-1.5 text-xs font-medium text-sky-300 hover:bg-sky-500/25"
                  >
                    Trigger
                  </button>
                  <button
                    type="button"
                    onClick={() => void openEdit(rule.id)}
                    class="rounded-lg border border-slate-700 px-3 py-1.5 text-xs text-slate-200 hover:bg-slate-800"
                  >
                    Edit TOML
                  </button>
                  <button
                    type="button"
                    onClick={() => void handleDelete(rule.id)}
                    class="rounded-lg border border-rose-500/40 px-2.5 py-1.5 text-xs text-rose-300 hover:bg-rose-500/15"
                  >
                    Delete
                  </button>
                </div>
              </div>
            )}
          </For>
        </Show>
      </div>

      {/* Execution History Table */}
      <section class="rounded-2xl border border-slate-800 bg-slate-900/70 p-5 space-y-3">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <h2 class="text-sm font-semibold uppercase tracking-wider text-slate-300">
            Execution History
          </h2>
          <div class="flex items-center gap-2">
            <select
              value={runFilter()}
              onChange={(e) => {
                setRunFilter(e.currentTarget.value);
                void loadRuns(e.currentTarget.value);
              }}
              class="rounded-lg border border-slate-700 bg-slate-950 px-2.5 py-1.5 text-xs text-white"
            >
              <option value="">All Automations</option>
              <For each={store.automations()}>
                {(a) => <option value={a.id}>{a.id}</option>}
              </For>
            </select>
            <button
              type="button"
              onClick={() => void loadRuns(runFilter())}
              class="rounded-lg border border-slate-700 px-2.5 py-1.5 text-xs text-slate-300 hover:bg-slate-800"
            >
              Refresh
            </button>
          </div>
        </div>

        <Show
          when={runs().length > 0}
          fallback={
            <p class="py-6 text-center text-xs text-slate-500">
              No automation runs recorded yet.
            </p>
          }
        >
          <div class="overflow-x-auto">
            <table class="w-full text-left text-xs">
              <thead>
                <tr class="border-b border-slate-800 uppercase text-slate-400">
                  <th class="py-2 pr-3">Started</th>
                  <th class="py-2 pr-3">Automation</th>
                  <th class="py-2 pr-3">Trigger</th>
                  <th class="py-2 pr-3">Outcome</th>
                  <th class="py-2 pr-3">Duration</th>
                  <th class="py-2">Error</th>
                </tr>
              </thead>
              <tbody class="divide-y divide-slate-800/70">
                <For each={runs()}>
                  {(run) => (
                    <tr>
                      <td class="py-2 pr-3 font-mono text-slate-400">
                        {new Date(run.started_at).toLocaleString()}
                      </td>
                      <td class="py-2 pr-3 font-mono text-slate-200">
                        {run.automation_id}
                      </td>
                      <td class="py-2 pr-3 text-slate-300">
                        {run.trigger_source}
                      </td>
                      <td class="py-2 pr-3">
                        <span
                          class={`rounded-full px-2 py-0.5 font-medium ${outcomeBadge(run.outcome)}`}
                        >
                          {run.outcome}
                        </span>
                      </td>
                      <td class="py-2 pr-3 font-mono text-slate-400">
                        {run.finished_at !== null
                          ? `${Math.max(0, run.finished_at - run.started_at)} ms`
                          : "—"}
                      </td>
                      <td class="py-2 font-mono text-rose-300">
                        {run.error ?? "—"}
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </section>
    </div>
  );
};
