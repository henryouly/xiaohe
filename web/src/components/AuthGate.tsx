import { createSignal, Show, type Component } from "solid-js";
import { ApiError } from "../lib/api";
import { useXiaohe } from "../lib/store";

export const AuthGate: Component = () => {
  const store = useXiaohe();
  const [username, setUsername] = createSignal("");
  const [displayName, setDisplayName] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [submitting, setSubmitting] = createSignal(false);

  const isBootstrap = () => !store.initialized();

  const onSubmit = async (e: SubmitEvent) => {
    e.preventDefault();
    setError(null);
    setSubmitting(true);
    try {
      if (isBootstrap()) {
        await store.bootstrap(
          username().trim(),
          password(),
          displayName().trim() || username().trim(),
        );
      } else {
        await store.login(username().trim(), password());
      }
    } catch (err) {
      if (err instanceof ApiError) {
        setError(err.message);
      } else {
        setError("Network error — could not reach Xiaohe Core");
      }
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div class="min-h-full flex items-center justify-center px-4 py-12 bg-gradient-to-b from-slate-950 via-slate-900 to-slate-950">
      <div class="w-full max-w-md rounded-2xl border border-slate-800 bg-slate-900/80 p-8 shadow-2xl backdrop-blur">
        <div class="mb-6 flex items-center gap-3">
          <div class="flex h-10 w-10 items-center justify-center rounded-xl bg-emerald-500/15 text-emerald-400 ring-1 ring-emerald-500/30 font-bold text-lg">
            X
          </div>
          <div>
            <h1 class="text-xl font-semibold text-white">Xiaohe</h1>
            <p class="text-xs text-slate-400">
              {isBootstrap()
                ? "First-boot setup · Create your Admin account"
                : "Sign in to your home dashboard"}
            </p>
          </div>
        </div>

        <Show when={error()}>
          <div class="mb-4 rounded-lg border border-rose-500/40 bg-rose-500/10 px-3 py-2 text-sm text-rose-200">
            {error()}
          </div>
        </Show>

        <form onSubmit={onSubmit} class="space-y-4">
          <div>
            <label class="mb-1.5 block text-xs font-medium uppercase tracking-wider text-slate-400">
              Username
            </label>
            <input
              type="text"
              required
              minLength={2}
              autocomplete="username"
              value={username()}
              onInput={(e) => setUsername(e.currentTarget.value)}
              placeholder="admin"
              class="w-full rounded-lg border border-slate-700 bg-slate-950 px-3.5 py-2.5 text-sm text-white placeholder-slate-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
            />
          </div>

          <Show when={isBootstrap()}>
            <div>
              <label class="mb-1.5 block text-xs font-medium uppercase tracking-wider text-slate-400">
                Display Name
              </label>
              <input
                type="text"
                value={displayName()}
                onInput={(e) => setDisplayName(e.currentTarget.value)}
                placeholder="Household Admin"
                class="w-full rounded-lg border border-slate-700 bg-slate-950 px-3.5 py-2.5 text-sm text-white placeholder-slate-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
              />
            </div>
          </Show>

          <div>
            <label class="mb-1.5 block text-xs font-medium uppercase tracking-wider text-slate-400">
              Password
            </label>
            <input
              type="password"
              required
              minLength={8}
              autocomplete={isBootstrap() ? "new-password" : "current-password"}
              value={password()}
              onInput={(e) => setPassword(e.currentTarget.value)}
              placeholder="At least 8 characters"
              class="w-full rounded-lg border border-slate-700 bg-slate-950 px-3.5 py-2.5 text-sm text-white placeholder-slate-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
            />
          </div>

          <button
            type="submit"
            disabled={submitting()}
            class="w-full rounded-lg bg-emerald-500 px-4 py-2.5 text-sm font-semibold text-slate-950 transition hover:bg-emerald-400 disabled:opacity-50"
          >
            {submitting()
              ? "Please wait…"
              : isBootstrap()
                ? "Create Admin Account"
                : "Sign In"}
          </button>
        </form>
      </div>
    </div>
  );
};
