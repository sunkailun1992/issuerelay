import { useEngineEvents } from "@/hooks/useEngineEvents";

/** Mounts the engine event subscription; it must sit inside `QueryClientProvider`. */
export function EngineEventsBridge() {
  useEngineEvents();
  return null;
}
