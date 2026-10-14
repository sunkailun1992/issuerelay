import { QueryClientProvider } from "@tanstack/react-query";
import { type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { HashRouter, Navigate, Route, Routes, useLocation } from "react-router";
import { EngineEventsBridge } from "@/components/EngineEventsBridge";
import { ErrorBoundary } from "@/components/ErrorBoundary";
import { FirstRunGate } from "@/components/FirstRunGate";
import { AppShell } from "@/components/layout/AppShell";
import { QuitDialog } from "@/components/QuitDialog";
import { StartupGate } from "@/components/StartupGate";
import { Toaster } from "@/components/ui/sonner";
import { useAppInfo, useSettings } from "@/hooks/queries";
import { resolveLanguage, setLanguage } from "@/i18n";
import type { Backend } from "@/lib/backend";
import { BackendProvider } from "@/lib/backend-context";
import { createQueryClient } from "@/lib/query-client";
import { useSystemTheme } from "@/lib/theme";
import Bindings from "@/pages/Bindings";
import Doctor from "@/pages/Doctor";
import Help from "@/pages/Help";
import Issues from "@/pages/Issues";
import Onboarding from "@/pages/onboarding/Onboarding";
import Overview from "@/pages/Overview";
import RunDetail from "@/pages/RunDetail";
import Runs from "@/pages/Runs";
import Settings from "@/pages/Settings";

/** Follows the system's light or dark appearance. */
function ThemeSync() {
  useSystemTheme();
  return null;
}

/**
 * Switches the interface to the language the settings ask for. The panel starts in the system
 * language (main.tsx, from the locale Rust reports); a language chosen in the settings is followed
 * once they have been read, and again whenever they change.
 */
function LanguageSync() {
  const { i18n } = useTranslation();
  const info = useAppInfo();
  const settings = useSettings();
  const wanted = info.data && settings.data ? resolveLanguage(settings.data.language, info.data.system_locale ?? undefined) : null;
  useEffect(() => {
    if (wanted !== null && wanted !== i18n.language) void setLanguage(wanted);
  }, [wanted, i18n.language]);
  return null;
}

/**
 * The boundary around the pages, inside the router: a page that fails is replaced by the crash
 * page, and going to another address clears it (the hash route is the reset key).
 */
function PageBoundary({ children }: { children: ReactNode }) {
  const location = useLocation();
  return <ErrorBoundary resetKey={`${location.pathname}${location.search}`}>{children}</ErrorBoundary>;
}

/**
 * The panel: the backend it talks to, the data cache, the engine's events, the pages inside the
 * shell and, outside the shell, the first-run guide. The start-failure page replaces the pages
 * (not the quit prompt and the toasts, which also work then). A render error shows the crash page
 * instead of a blank window: for a page, in place of the pages (another address recovers); for
 * the parts around them, in place of the panel. The quit prompt and the toasts are outside both,
 * each in a boundary of its own that shows nothing if it fails: quitting must still ask while runs
 * are in progress, whatever page failed.
 */
export default function App({ backend }: { backend: Backend }) {
  const [client] = useState(createQueryClient);
  return (
    <ErrorBoundary>
      <BackendProvider backend={backend}>
        <QueryClientProvider client={client}>
          <ErrorBoundary>
            <EngineEventsBridge />
            <ThemeSync />
            <LanguageSync />
            <StartupGate>
              <HashRouter>
                <PageBoundary>
                  <Routes>
                    <Route path="/onboarding" element={<Onboarding />} />
                    <Route element={<FirstRunGate />}>
                      <Route element={<AppShell />}>
                        <Route index element={<Overview />} />
                        <Route path="issues" element={<Issues />} />
                        <Route path="runs" element={<Runs />} />
                        <Route path="runs/:id" element={<RunDetail />} />
                        <Route path="bindings" element={<Bindings />} />
                        <Route path="doctor" element={<Doctor />} />
                        <Route path="settings" element={<Settings />} />
                        <Route path="help" element={<Help />} />
                        <Route path="*" element={<Navigate to="/" replace />} />
                      </Route>
                    </Route>
                  </Routes>
                </PageBoundary>
              </HashRouter>
            </StartupGate>
          </ErrorBoundary>
          <ErrorBoundary fallback="nothing">
            <QuitDialog />
          </ErrorBoundary>
          <ErrorBoundary fallback="nothing">
            <Toaster />
          </ErrorBoundary>
        </QueryClientProvider>
      </BackendProvider>
    </ErrorBoundary>
  );
}
