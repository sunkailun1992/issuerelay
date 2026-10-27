import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef } from "react";
import type { LinearSetup } from "@/lib/types";
import { queryKeys, useStatus } from "./queries";

/**
 * What only has an answer with a connection to Linear. A page that read it while Linear was still
 * connecting, or before the login, holds an error (or the answer for another account) that no
 * event replaces, so it is read again whenever the connection changes.
 */
const NEEDS_LINEAR = [queryKeys.issues, queryKeys.projects, queryKeys.labels, queryKeys.doctor] as const;

/**
 * Reads what needs Linear again when the connection to it changes: its kind in the status
 * (`LinearSetup`) goes from "connecting" to "connected" or "connect_failed", from "connected" to
 * "not_configured" after a logout, and so on, whichever page asked and however the change came
 * about (the host's first connection, a login, a reconnect). Only a change counts, not the first
 * time the status is read, and not a status that is read again as it was: a Status event every
 * few seconds must not make the Linear calls and the environment check run again.
 * The status itself is refreshed by the engine events (`useEngineEvents`). Must run inside
 * `QueryClientProvider` and `BackendProvider`.
 */
export function useLinearConnectionRefresh(): void {
  const client = useQueryClient();
  const kind = useStatus().data?.linear.kind;
  const known = useRef<LinearSetup["kind"] | undefined>(undefined);
  useEffect(() => {
    if (kind === undefined) return;
    const before = known.current;
    known.current = kind;
    if (before === undefined || before === kind) return;
    for (const queryKey of NEEDS_LINEAR) void client.invalidateQueries({ queryKey });
  }, [kind, client]);
}
