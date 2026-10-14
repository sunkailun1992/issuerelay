import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { useBackend } from "@/lib/backend-context";
import { queryKeys } from "./queries";
import { subscription } from "./subscription";

/**
 * Keeps what the panel shows in step with the engine, which tells what changed rather than what
 * it is now: a changed job refreshes the lists and that job's pages, a changed setup the status
 * (it carries the Linear connection, so "connecting" turns into "connected" without a reload),
 * a changed configuration the settings and bindings. When events were lost (`resync`) everything
 * is read again. Must run inside `QueryClientProvider` and `BackendProvider`.
 */
export function useEngineEvents(): void {
  const backend = useBackend();
  const client = useQueryClient();
  useEffect(() => {
    // `client.invalidateQueries` is looked up when an event arrives, not here.
    const refresh = (...keys: readonly (readonly unknown[])[]) => {
      for (const queryKey of keys) void client.invalidateQueries({ queryKey });
    };
    const stopEvents = subscription(
      backend.onEngineEvent((event) => {
        switch (event.kind) {
          case "status":
            refresh(queryKeys.status);
            break;
          case "job":
            refresh(queryKeys.status, queryKeys.jobs, queryKeys.job(event.job.id), queryKeys.log(event.job.id), queryKeys.issues);
            break;
          case "config":
            refresh(queryKeys.settings, queryKeys.bindings, queryKeys.status);
            break;
        }
      }),
    );
    const stopResync = subscription(backend.onResync(() => void client.invalidateQueries()));
    return () => {
      stopEvents();
      stopResync();
    };
  }, [backend, client]);
}
