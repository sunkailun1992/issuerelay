import { createContext, type ReactNode, useContext } from "react";
import type { Backend } from "./backend";

const BackendContext = createContext<Backend | null>(null);

/**
 * Gives the components their backend: the real commands in the app, the mock in a browser and in
 * tests. There is no global default, so a component that forgot its provider fails loudly.
 */
export function BackendProvider({ backend, children }: { backend: Backend; children: ReactNode }) {
  return <BackendContext value={backend}>{children}</BackendContext>;
}

/** The backend of the nearest `BackendProvider`; throws without one. */
// eslint-disable-next-line react-refresh/only-export-components -- the provider and its hook belong together; a change here reloads the page.
export function useBackend(): Backend {
  const backend = useContext(BackendContext);
  if (backend === null) throw new Error("BackendProvider is missing");
  return backend;
}
