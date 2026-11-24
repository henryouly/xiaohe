import { Navigate, Route, Router } from "@solidjs/router";
import { Show, type Component, type ParentComponent } from "solid-js";
import { AppShell } from "./components/AppShell";
import { AuthGate } from "./components/AuthGate";
import { useXiaohe, XiaoheProvider } from "./lib/store";

const PlaceholderView: Component<{ title: string; subtitle: string }> = (props) => (
  <div class="rounded-2xl border border-slate-800 bg-slate-900/60 p-6">
    <h1 class="text-xl font-semibold text-white">{props.title}</h1>
    <p class="mt-1 text-sm text-slate-400">{props.subtitle}</p>
  </div>
);

const AdminOnly: ParentComponent = (props) => {
  const store = useXiaohe();
  return (
    <Show when={store.isAdmin()} fallback={<Navigate href="/" />}>
      {props.children}
    </Show>
  );
};

const ProtectedShell: ParentComponent = (props) => {
  const store = useXiaohe();
  return (
    <Show
      when={!store.authLoading()}
      fallback={
        <div class="min-h-full flex items-center justify-center bg-slate-950 text-slate-400 text-sm">
          Connecting to Xiaohe Core…
        </div>
      }
    >
      <Show when={store.user()} fallback={<AuthGate />}>
        <AppShell>{props.children}</AppShell>
      </Show>
    </Show>
  );
};

export const App: Component = () => {
  return (
    <XiaoheProvider>
      <Router root={ProtectedShell}>
        <Route
          path="/"
          component={() => (
            <PlaceholderView
              title="Area Control Dashboard"
              subtitle="Phase 6.2 view coming next."
            />
          )}
        />
        <Route
          path="/history"
          component={() => (
            <PlaceholderView
              title="History & Timeline"
              subtitle="Phase 6.2 view coming next."
            />
          )}
        />
        <Route
          path="/skills"
          component={() => (
            <AdminOnly>
              <PlaceholderView
                title="Skill Manager"
                subtitle="Phase 6.2 view coming next."
              />
            </AdminOnly>
          )}
        />
        <Route
          path="/automations"
          component={() => (
            <AdminOnly>
              <PlaceholderView
                title="Automations & Run Logs"
                subtitle="Phase 6.2 view coming next."
              />
            </AdminOnly>
          )}
        />
        <Route path="*" component={() => <Navigate href="/" />} />
      </Router>
    </XiaoheProvider>
  );
};
