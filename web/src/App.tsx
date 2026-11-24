import { Navigate, Route, Router } from "@solidjs/router";
import { Show, type Component, type ParentComponent } from "solid-js";
import { AppShell } from "./components/AppShell";
import { AuthGate } from "./components/AuthGate";
import { useXiaohe, XiaoheProvider } from "./lib/store";
import { AutomationsView } from "./views/AutomationsView";
import { DashboardView } from "./views/DashboardView";
import { HistoryView } from "./views/HistoryView";
import { SkillsView } from "./views/SkillsView";

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
        <Route path="/" component={DashboardView} />
        <Route path="/history" component={HistoryView} />
        <Route
          path="/skills"
          component={() => (
            <AdminOnly>
              <SkillsView />
            </AdminOnly>
          )}
        />
        <Route
          path="/automations"
          component={() => (
            <AdminOnly>
              <AutomationsView />
            </AdminOnly>
          )}
        />
        <Route path="*" component={() => <Navigate href="/" />} />
      </Router>
    </XiaoheProvider>
  );
};
