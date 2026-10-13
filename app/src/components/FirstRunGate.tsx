import type { ReactNode } from "react";
import { Navigate, Outlet } from "react-router";
import { useSettings } from "@/hooks/queries";
import { ErrorState, LoadingState } from "./states";

/** The gate sits outside the shell, so what it shows instead of a page needs its own margins. */
function Bare({ children }: { children: ReactNode }) {
  return <main className="mx-auto w-full max-w-(--content-max-width) p-(--page-padding)">{children}</main>;
}

/**
 * Sends a first run to the guide: the pages below show only once the first-run guide has been
 * completed (`onboarding_completed`). While the settings are being read nothing of the page
 * shows, and settings that cannot be read are an error with a retry, not a page on a guess.
 */
export function FirstRunGate() {
  const settings = useSettings();
  if (settings.isError) {
    return (
      <Bare>
        <ErrorState error={settings.error} onRetry={() => void settings.refetch()} />
      </Bare>
    );
  }
  if (settings.data === undefined) {
    return (
      <Bare>
        <LoadingState />
      </Bare>
    );
  }
  if (!settings.data.onboarding_completed) return <Navigate to="/onboarding" replace />;
  return <Outlet />;
}
