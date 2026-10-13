import {
  CircleHelp,
  FolderGit2,
  History,
  LayoutDashboard,
  ListTodo,
  Settings,
  Stethoscope,
  Workflow,
  type LucideIcon,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { NavLink, Outlet } from "react-router";
import { useAppInfo, useStatus } from "@/hooks/queries";
import { trayState } from "@/lib/problem";
import type { StatusView } from "@/lib/types";
import { cn } from "@/lib/utils";

/** The pages of the sidebar, in order; `key` is the page's name in `nav.*`. */
const PAGES: { to: string; key: string; icon: LucideIcon; end?: boolean }[] = [
  { to: "/", key: "overview", icon: LayoutDashboard, end: true },
  { to: "/issues", key: "issues", icon: ListTodo },
  { to: "/runs", key: "runs", icon: History },
  { to: "/bindings", key: "bindings", icon: FolderGit2 },
  { to: "/doctor", key: "doctor", icon: Stethoscope },
  { to: "/settings", key: "settings", icon: Settings },
  { to: "/help", key: "help", icon: CircleHelp },
];

/**
 * The line at the foot of the sidebar, from structured data only: waiting for runs to finish
 * comes first, then the first connection to Linear still being under way (no problem yet), then
 * what the tray shows (`trayState`, the same rule and the same cases).
 */
function monitoring({ status, linear }: StatusView): { key: string; failed: boolean } {
  if (status.draining) return { key: "shell.draining", failed: false };
  if (linear.kind === "connecting") return { key: "shell.connecting", failed: false };
  switch (trayState(status, linear)) {
    case "problem":
      return { key: "shell.problem", failed: true };
    case "paused":
      return { key: "shell.paused", failed: false };
    default:
      return { key: "shell.monitoring", failed: false };
  }
}

/** The frame of every page: the sidebar with the pages, the monitoring state and the version, and the page itself. */
export function AppShell() {
  const { t } = useTranslation();
  const status = useStatus();
  const info = useAppInfo();
  const line = status.data ? monitoring(status.data) : null;
  return (
    <div className="flex h-screen bg-background text-foreground">
      <aside className="flex w-(--sidebar-width) shrink-0 flex-col border-r border-sidebar-border bg-sidebar text-sidebar-foreground">
        <div className="flex items-center gap-2 p-4">
          <span className="flex size-8 items-center justify-center rounded-lg bg-sidebar-primary text-sidebar-primary-foreground">
            <Workflow aria-hidden className="size-4" />
          </span>
          <span className="text-base font-semibold">{t("app.name")}</span>
        </div>
        <nav aria-label={t("app.name")} className="flex-1 px-2">
          <ul className="flex flex-col gap-1">
            {PAGES.map(({ to, key, icon: Icon, end }) => (
              <li key={to}>
                <NavLink
                  to={to}
                  end={end}
                  className={({ isActive }) =>
                    cn(
                      "flex items-center gap-2 rounded-md px-3 py-2 outline-none transition-colors focus-visible:ring-3 focus-visible:ring-sidebar-ring/50",
                      isActive
                        ? "bg-sidebar-accent font-medium text-sidebar-accent-foreground"
                        : "text-sidebar-foreground/70 hover:bg-sidebar-accent/60 hover:text-sidebar-accent-foreground",
                    )
                  }
                >
                  <Icon aria-hidden className="size-4" />
                  {t(`nav.${key}`)}
                </NavLink>
              </li>
            ))}
          </ul>
        </nav>
        <div className="flex flex-col gap-1 border-t border-sidebar-border p-4 text-xs">
          {line && <p className={cn("font-medium", line.failed && "text-state-failed")}>{t(line.key)}</p>}
          {info.data && <p className="text-muted-foreground">{t("shell.version", { version: info.data.version })}</p>}
        </div>
      </aside>
      <main className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto w-full max-w-(--content-max-width) px-(--page-padding) py-6">
          <Outlet />
        </div>
      </main>
    </div>
  );
}
