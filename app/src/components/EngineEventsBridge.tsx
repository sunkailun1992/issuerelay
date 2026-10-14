import { useEngineEvents } from "@/hooks/useEngineEvents";
import { useLinearConnectionRefresh } from "@/hooks/useLinearConnectionRefresh";

/**
 * Mounts what keeps the panel's data in step with the engine: the event subscription, and the
 * refresh of what needs Linear when the connection to it changes. It must sit inside
 * `QueryClientProvider`.
 */
export function EngineEventsBridge() {
  useEngineEvents();
  useLinearConnectionRefresh();
  return null;
}
