import { useQueryClient } from "@tanstack/react-query";
import { type ReactNode, useEffect } from "react";
import { queryKeys, useStartupState } from "@/hooks/queries";
import { subscription } from "@/hooks/subscription";
import { useBackend } from "@/lib/backend-context";
import { StartupError } from "./StartupError";

/**
 * The startup error page instead of the panel, for any failed start of the host. The state is
 * asked for (`startup_state`, again every second while starting), so a panel opened after the
 * failure sees it too; `startup-failed` updates a panel that was already open. "Ready" means the
 * host started; Linear may still be connecting, which `LinearStatus` shows.
 */
export function StartupGate({ children }: { children: ReactNode }) {
  const backend = useBackend();
  const client = useQueryClient();
  const startup = useStartupState();
  useEffect(
    () => subscription(backend.onStartupFailed((state) => client.setQueryData(queryKeys.startup, state))),
    [backend, client],
  );
  if (startup.data?.kind === "failed") {
    return <StartupError code={startup.data.code} detail={startup.data.detail} />;
  }
  return children;
}
