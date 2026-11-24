import { A, useLocation } from "@solidjs/router";
import { For, Show, type ParentComponent } from "solid-js";
import { useXiaohe } from "../lib/store";

interface NavItem {
  href: string;
  label: string;
  shortLabel: string;
  adminOnly?: boolean;
}

const NAV_ITEMS: NavItem[] = [
  { href: "/", label: "Dashboard", shortLabel: "Home" },
  { href: "/history", label: "History", shortLabel: "History" },
  { href: "/skills", label: "Skills", shortLabel: "Skills", adminOnly: true },
  {
    href: "/automations",
    label: "Automations",
    shortLabel: "Auto",
    adminOnly: true,
  },
];

export const AppShell: ParentComponent = (props) => {
  const store = useXiaohe();
  const location = useLocation();

  const visibleNav = () =>
    NAV_ITEMS.filter((item) => !item.adminOnly || store.isAdmin());

  const isActive = (href: string) =>
    href === "/"
      ? location.pathname === "/"
      : location.pathname.startsWith(href);

  const statusColor = () => {
    switch (store.sseStatus()) {
      case "connected":
        return "bg-emerald-400";
      case "connecting":
      case "reconnecting":
        return "bg-amber-400 animate-pulse";
      default:
        return "bg-slate-500";
    }
  };

  return (
    <div class="min-h-full flex flex-col md:flex-row bg-slate-950 text-slate-100">
      {/* Desktop Sidebar */}
      <aside class="hidden md:flex md:w-60 md:flex-col md:border-r md:border-slate-800/80 md:bg-slate-900/50">
        <div class="flex items-center gap-3 px-5 py-5 border-b border-slate-800/80">
          <div class="flex h-9 w-9 items-center justify-center rounded-xl bg-emerald-500/15 text-emerald-400 ring-1 ring-emerald-500/30 font-bold">
            X
          </div>
          <div class="min-w-0">
            <div class="font-semibold text-white leading-tight">Xiaohe</div>
            <div class="flex items-center gap-1.5 text-xs text-slate-400">
              <span class={`inline-block h-2 w-2 rounded-full ${statusColor()}`} />
              <span class="capitalize">{store.sseStatus()}</span>
            </div>
          </div>
        </div>

        <nav class="flex-1 space-y-1 p-3">
          <For each={visibleNav()}>
            {(item) => (
              <A
                href={item.href}
                class={`flex items-center justify-between rounded-lg px-3.5 py-2.5 text-sm font-medium transition ${
                  isActive(item.href)
                    ? "bg-emerald-500/15 text-emerald-300 ring-1 ring-emerald-500/30"
                    : "text-slate-300 hover:bg-slate-800/70 hover:text-white"
                }`}
              >
                <span>{item.label}</span>
                <Show when={item.adminOnly}>
                  <span class="rounded bg-slate-800 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-slate-400">
                    Admin
                  </span>
                </Show>
              </A>
            )}
          </For>
        </nav>

        <div class="border-t border-slate-800/80 p-4">
          <div class="flex items-center justify-between gap-2">
            <div class="min-w-0">
              <div class="truncate text-sm font-medium text-white">
                {store.user()?.display_name}
              </div>
              <div class="text-xs uppercase tracking-wider text-slate-400">
                {store.user()?.role}
              </div>
            </div>
            <button
              type="button"
              onClick={() => void store.logout()}
              class="rounded-lg border border-slate-700 px-2.5 py-1.5 text-xs font-medium text-slate-300 hover:bg-slate-800 hover:text-white"
            >
              Sign out
            </button>
          </div>
        </div>
      </aside>

      {/* Mobile Top Bar */}
      <header class="flex md:hidden items-center justify-between border-b border-slate-800 bg-slate-900/80 px-4 py-3 backdrop-blur sticky top-0 z-20">
        <div class="flex items-center gap-2.5">
          <div class="flex h-8 w-8 items-center justify-center rounded-lg bg-emerald-500/15 text-emerald-400 ring-1 ring-emerald-500/30 font-bold text-sm">
            X
          </div>
          <span class="font-semibold text-white">Xiaohe</span>
          <span class={`inline-block h-2 w-2 rounded-full ${statusColor()}`} />
        </div>
        <div class="flex items-center gap-2">
          <span class="rounded bg-slate-800 px-2 py-0.5 text-[11px] uppercase text-slate-300">
            {store.user()?.role}
          </span>
          <button
            type="button"
            onClick={() => void store.logout()}
            class="rounded-md border border-slate-700 px-2 py-1 text-xs text-slate-300"
          >
            Sign out
          </button>
        </div>
      </header>

      {/* Main Content */}
      <main class="flex-1 overflow-y-auto px-4 py-5 pb-24 md:px-8 md:py-7 md:pb-8">
        <div class="mx-auto max-w-6xl">{props.children}</div>
      </main>

      {/* Mobile Bottom Nav */}
      <nav class="fixed bottom-0 inset-x-0 z-20 flex md:hidden border-t border-slate-800 bg-slate-900/95 backdrop-blur">
        <For each={visibleNav()}>
          {(item) => (
            <A
              href={item.href}
              class={`flex-1 py-3 text-center text-xs font-medium transition ${
                isActive(item.href)
                  ? "text-emerald-400 border-t-2 border-emerald-400 -mt-px"
                  : "text-slate-400"
              }`}
            >
              {item.shortLabel}
            </A>
          )}
        </For>
      </nav>

      {/* Toast Stack */}
      <div class="fixed right-4 bottom-20 md:bottom-5 z-50 flex flex-col gap-2 max-w-sm">
        <For each={store.toasts()}>
          {(toast) => (
            <div
              class={`rounded-xl border px-3.5 py-2.5 text-sm shadow-lg backdrop-blur ${
                toast.kind === "error"
                  ? "border-rose-500/40 bg-rose-950/90 text-rose-100"
                  : toast.kind === "success"
                    ? "border-emerald-500/40 bg-emerald-950/90 text-emerald-100"
                    : "border-slate-700 bg-slate-900/90 text-slate-100"
              }`}
            >
              {toast.text}
            </div>
          )}
        </For>
      </div>
    </div>
  );
};
