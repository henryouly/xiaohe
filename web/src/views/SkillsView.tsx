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
  type SkillDetailResponse,
  type SkillLogEntry,
  type SkillSummary,
} from "../lib/api";
import { useXiaohe } from "../lib/store";

export const SkillsView: Component = () => {
  const store = useXiaohe();
  const [busySkill, setBusySkill] = createSignal<string | null>(null);

  // Config & Secrets editor state
  const [editingSkill, setEditingSkill] = createSignal<SkillDetailResponse | null>(
    null,
  );
  const [configText, setConfigText] = createSignal("{}");
  const [enabledDraft, setEnabledDraft] = createSignal(true);
  const [secretDrafts, setSecretDrafts] = createSignal<Record<string, string>>({});
  const [savingConfig, setSavingConfig] = createSignal(false);

  // Logs drawer state
  const [logsSkillId, setLogsSkillId] = createSignal<string | null>(null);
  const [logs, setLogs] = createSignal<SkillLogEntry[]>([]);
  const [loadingLogs, setLoadingLogs] = createSignal(false);

  // Action invoke state
  const [invokeSkillId, setInvokeSkillId] = createSignal<string | null>(null);
  const [invokeAction, setInvokeAction] = createSignal("");
  const [invokeParams, setInvokeParams] = createSignal("{}");
  const [invokeResult, setInvokeResult] = createSignal<string | null>(null);

  onMount(() => {
    void store.refreshAdminData();
  });

  const runLifecycle = async (
    skillId: string,
    op: "start" | "stop" | "restart" | "reinstall",
  ) => {
    setBusySkill(skillId);
    try {
      await api.skillLifecycle(skillId, op);
      await store.refreshAdminData();
      store.notify("success", `${skillId}: ${op} completed`);
    } catch (err) {
      store.notify(
        "error",
        err instanceof ApiError ? err.message : `${op} failed`,
      );
    } finally {
      setBusySkill(null);
    }
  };

  const openConfigEditor = async (skillId: string) => {
    try {
      const detail = await api.getSkill(skillId);
      setEditingSkill(detail);
      setEnabledDraft(detail.enabled);
      setConfigText(JSON.stringify(detail.config ?? {}, null, 2));
      const initialSecrets: Record<string, string> = {};
      for (const k of Object.keys(detail.secrets ?? {})) {
        initialSecrets[k] = "";
      }
      setSecretDrafts(initialSecrets);
    } catch (err) {
      store.notify(
        "error",
        err instanceof ApiError ? err.message : "Failed to load skill details",
      );
    }
  };

  const saveConfig = async (e: SubmitEvent) => {
    e.preventDefault();
    const detail = editingSkill();
    if (!detail) return;

    let parsedConfig: Record<string, unknown>;
    try {
      parsedConfig = JSON.parse(configText()) as Record<string, unknown>;
    } catch {
      store.notify("error", "Config must be valid JSON");
      return;
    }

    const nonEmptySecrets: Record<string, string> = {};
    for (const [k, v] of Object.entries(secretDrafts())) {
      if (v.trim().length > 0) {
        nonEmptySecrets[k] = v;
      }
    }

    setSavingConfig(true);
    try {
      await api.updateSkillConfig(detail.skill_id, {
        enabled: enabledDraft(),
        config: parsedConfig,
        secrets: nonEmptySecrets,
      });
      setEditingSkill(null);
      await store.refreshAdminData();
      store.notify("success", `${detail.skill_id} configuration saved`);
    } catch (err) {
      store.notify(
        "error",
        err instanceof ApiError ? err.message : "Failed to save config",
      );
    } finally {
      setSavingConfig(false);
    }
  };

  const openLogs = async (skillId: string) => {
    setLogsSkillId(skillId);
    setLoadingLogs(true);
    try {
      const res = await api.skillLogs(skillId, 200);
      setLogs(res.lines);
    } catch (err) {
      store.notify(
        "error",
        err instanceof ApiError ? err.message : "Failed to fetch logs",
      );
    } finally {
      setLoadingLogs(false);
    }
  };

  const runInvoke = async (e: SubmitEvent) => {
    e.preventDefault();
    const id = invokeSkillId();
    if (!id) return;
    let params: Record<string, unknown> = {};
    try {
      params = JSON.parse(invokeParams()) as Record<string, unknown>;
    } catch {
      store.notify("error", "Params must be valid JSON");
      return;
    }
    try {
      const res = await api.invokeSkill(id, invokeAction().trim(), params);
      setInvokeResult(JSON.stringify(res.result, null, 2));
      store.notify("success", `Invoked ${id}.${invokeAction()}`);
    } catch (err) {
      store.notify(
        "error",
        err instanceof ApiError ? err.message : "Invoke failed",
      );
    }
  };

  const statusBadge = (status: SkillSummary["status"]) => {
    switch (status) {
      case "running":
        return "bg-emerald-500/15 text-emerald-300";
      case "installing":
        return "bg-sky-500/15 text-sky-300";
      case "error":
        return "bg-rose-500/20 text-rose-300";
      default:
        return "bg-slate-800 text-slate-400";
    }
  };

  return (
    <div class="space-y-6">
      <div class="flex items-center justify-between">
        <div>
          <h1 class="text-2xl font-semibold text-white">Skill Manager</h1>
          <p class="text-sm text-slate-400">
            Isolated Python Skills managed via xiaohe-supervisor & uv
          </p>
        </div>
        <button
          type="button"
          onClick={() => void store.refreshAdminData()}
          class="rounded-lg border border-slate-700 bg-slate-900 px-3.5 py-2 text-xs font-medium text-slate-200 hover:bg-slate-800"
        >
          Refresh
        </button>
      </div>

      <Show
        when={store.skills().length > 0}
        fallback={
          <div class="rounded-2xl border border-dashed border-slate-800 p-10 text-center text-sm text-slate-400">
            No skills discovered or xiaohe-supervisor is disconnected.
          </div>
        }
      >
        <div class="grid grid-cols-1 gap-4 lg:grid-cols-2">
          <For each={store.skills()}>
            {(skill) => (
              <div class="rounded-2xl border border-slate-800 bg-slate-900/75 p-5 space-y-4">
                <div class="flex items-start justify-between gap-3">
                  <div>
                    <div class="flex items-center gap-2">
                      <h2 class="text-base font-semibold text-white">
                        {skill.name}
                      </h2>
                      <span class="rounded bg-slate-800 px-2 py-0.5 text-[10px] uppercase tracking-wider text-slate-400">
                        {skill.mode}
                      </span>
                    </div>
                    <div class="text-xs font-mono text-slate-500">
                      {skill.skill_id} · v{skill.version} · {skill.entity_count}{" "}
                      entities · restarts: {skill.restart_count}
                    </div>
                  </div>
                  <span
                    class={`rounded-full px-2.5 py-0.5 text-xs font-medium capitalize ${statusBadge(skill.status)}`}
                  >
                    {skill.status}
                  </span>
                </div>

                <Show when={skill.last_error}>
                  <div class="rounded-lg border border-rose-500/30 bg-rose-950/50 px-3 py-2 text-xs font-mono text-rose-200">
                    {skill.last_error}
                  </div>
                </Show>

                <div class="flex flex-wrap gap-2">
                  <button
                    type="button"
                    disabled={busySkill() === skill.skill_id}
                    onClick={() => void runLifecycle(skill.skill_id, "start")}
                    class="rounded-lg bg-emerald-500/15 px-3 py-1.5 text-xs font-medium text-emerald-300 hover:bg-emerald-500/25 disabled:opacity-50"
                  >
                    Start
                  </button>
                  <button
                    type="button"
                    disabled={busySkill() === skill.skill_id}
                    onClick={() => void runLifecycle(skill.skill_id, "stop")}
                    class="rounded-lg bg-slate-800 px-3 py-1.5 text-xs font-medium text-slate-200 hover:bg-slate-700 disabled:opacity-50"
                  >
                    Stop
                  </button>
                  <button
                    type="button"
                    disabled={busySkill() === skill.skill_id}
                    onClick={() => void runLifecycle(skill.skill_id, "restart")}
                    class="rounded-lg bg-slate-800 px-3 py-1.5 text-xs font-medium text-slate-200 hover:bg-slate-700 disabled:opacity-50"
                  >
                    Restart
                  </button>
                  <button
                    type="button"
                    disabled={busySkill() === skill.skill_id}
                    onClick={() => void runLifecycle(skill.skill_id, "reinstall")}
                    class="rounded-lg bg-sky-500/15 px-3 py-1.5 text-xs font-medium text-sky-300 hover:bg-sky-500/25 disabled:opacity-50"
                  >
                    Reinstall (uv)
                  </button>
                  <button
                    type="button"
                    onClick={() => void openConfigEditor(skill.skill_id)}
                    class="rounded-lg border border-slate-700 px-3 py-1.5 text-xs font-medium text-slate-200 hover:bg-slate-800"
                  >
                    Config & Secrets
                  </button>
                  <button
                    type="button"
                    onClick={() => void openLogs(skill.skill_id)}
                    class="rounded-lg border border-slate-700 px-3 py-1.5 text-xs font-medium text-slate-200 hover:bg-slate-800"
                  >
                    Logs
                  </button>
                  <Show when={skill.mode === "action"}>
                    <button
                      type="button"
                      onClick={() => {
                        setInvokeSkillId(skill.skill_id);
                        setInvokeAction("");
                        setInvokeParams("{}");
                        setInvokeResult(null);
                      }}
                      class="rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-1.5 text-xs font-medium text-amber-200 hover:bg-amber-500/20"
                    >
                      Invoke
                    </button>
                  </Show>
                </div>
              </div>
            )}
          </For>
        </div>
      </Show>

      {/* Config & Write-Only Secrets Editor */}
      <Show when={editingSkill()}>
        {(detail) => (
          <form
            onSubmit={saveConfig}
            class="rounded-2xl border border-slate-700 bg-slate-900 p-5 space-y-4"
          >
            <div class="flex items-center justify-between">
              <h3 class="text-base font-semibold text-white">
                Configure {detail().skill_id}
              </h3>
              <button
                type="button"
                onClick={() => setEditingSkill(null)}
                class="text-xs text-slate-400 hover:text-white"
              >
                Close
              </button>
            </div>

            <label class="flex items-center gap-2 text-sm text-slate-200">
              <input
                type="checkbox"
                checked={enabledDraft()}
                onChange={(e) => setEnabledDraft(e.currentTarget.checked)}
                class="accent-emerald-500"
              />
              <span>Enabled (auto-start on boot)</span>
            </label>

            <div>
              <label class="mb-1 block text-xs uppercase tracking-wider text-slate-400">
                Non-Secret Config (JSON)
              </label>
              <textarea
                rows={6}
                value={configText()}
                onInput={(e) => setConfigText(e.currentTarget.value)}
                class="w-full rounded-lg border border-slate-700 bg-slate-950 p-3 font-mono text-xs text-slate-100"
              />
            </div>

            <Show when={Object.keys(detail().secrets ?? {}).length > 0}>
              <div class="space-y-2.5">
                <div class="text-xs uppercase tracking-wider text-slate-400">
                  Write-Only Secrets (leave blank to keep existing value)
                </div>
                <For each={Object.entries(detail().secrets)}>
                  {([key, meta]) => (
                    <div class="flex flex-col gap-1 sm:flex-row sm:items-center">
                      <div class="w-48 font-mono text-xs text-slate-300">
                        {key}{" "}
                        <span
                          class={`ml-1 rounded px-1.5 py-0.5 text-[10px] ${
                            meta.is_set
                              ? "bg-emerald-500/15 text-emerald-300"
                              : "bg-slate-800 text-slate-400"
                          }`}
                        >
                          {meta.is_set ? "set" : "unset"}
                        </span>
                      </div>
                      <input
                        type="password"
                        placeholder={
                          meta.is_set ? "•••••••• (unchanged)" : "Enter secret"
                        }
                        value={secretDrafts()[key] ?? ""}
                        onInput={(e) =>
                          setSecretDrafts((prev) => ({
                            ...prev,
                            [key]: e.currentTarget.value,
                          }))
                        }
                        class="flex-1 rounded-lg border border-slate-700 bg-slate-950 px-3 py-1.5 text-xs text-white"
                      />
                    </div>
                  )}
                </For>
              </div>
            </Show>

            <div class="flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setEditingSkill(null)}
                class="rounded-lg border border-slate-700 px-3.5 py-2 text-xs text-slate-300"
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={savingConfig()}
                class="rounded-lg bg-emerald-500 px-4 py-2 text-xs font-semibold text-slate-950 hover:bg-emerald-400 disabled:opacity-50"
              >
                {savingConfig() ? "Saving…" : "Save & Reload"}
              </button>
            </div>
          </form>
        )}
      </Show>

      {/* Logs Viewer */}
      <Show when={logsSkillId()}>
        {(skillId) => (
          <div class="rounded-2xl border border-slate-700 bg-slate-950 p-4 space-y-3">
            <div class="flex items-center justify-between">
              <div class="text-sm font-semibold text-white">
                Logs · <span class="font-mono">{skillId()}</span>
              </div>
              <div class="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => void openLogs(skillId())}
                  class="rounded border border-slate-700 px-2.5 py-1 text-xs text-slate-300 hover:bg-slate-800"
                >
                  Refresh
                </button>
                <button
                  type="button"
                  onClick={() => setLogsSkillId(null)}
                  class="text-xs text-slate-400 hover:text-white"
                >
                  Close
                </button>
              </div>
            </div>
            <Show
              when={!loadingLogs()}
              fallback={<div class="text-xs text-slate-500">Loading logs…</div>}
            >
              <pre class="max-h-72 overflow-y-auto rounded-lg bg-black/60 p-3 font-mono text-xs text-slate-300 whitespace-pre-wrap">
                {logs().length > 0
                  ? logs()
                      .map(
                        (l) =>
                          `[${new Date(l.ts).toISOString()}] ${l.line}`,
                      )
                      .join("\n")
                  : "(no stderr output captured)"}
              </pre>
            </Show>
          </div>
        )}
      </Show>

      {/* Action Skill Invoke Panel */}
      <Show when={invokeSkillId()}>
        {(skillId) => (
          <form
            onSubmit={runInvoke}
            class="rounded-2xl border border-amber-500/30 bg-slate-900 p-5 space-y-3"
          >
            <div class="flex items-center justify-between">
              <div class="text-sm font-semibold text-white">
                Invoke Action Skill · <span class="font-mono">{skillId()}</span>
              </div>
              <button
                type="button"
                onClick={() => setInvokeSkillId(null)}
                class="text-xs text-slate-400 hover:text-white"
              >
                Close
              </button>
            </div>
            <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <input
                type="text"
                required
                placeholder="action name (e.g. optimize)"
                value={invokeAction()}
                onInput={(e) => setInvokeAction(e.currentTarget.value)}
                class="rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-xs text-white"
              />
              <input
                type="text"
                placeholder='{"dry_run": true}'
                value={invokeParams()}
                onInput={(e) => setInvokeParams(e.currentTarget.value)}
                class="rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 font-mono text-xs text-white"
              />
            </div>
            <button
              type="submit"
              class="rounded-lg bg-amber-400 px-4 py-1.5 text-xs font-semibold text-slate-950 hover:bg-amber-300"
            >
              Run Invoke
            </button>
            <Show when={invokeResult()}>
              <pre class="overflow-x-auto rounded-lg bg-black/60 p-3 font-mono text-xs text-emerald-300">
                {invokeResult()}
              </pre>
            </Show>
          </form>
        )}
      </Show>
    </div>
  );
};
