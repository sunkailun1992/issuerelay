import { QueryClient } from "@tanstack/react-query";

/**
 * The data cache of the panel, made here for the app and for the tests alike, so that no test runs
 * by other rules than the app.
 *
 * Every call is a local command or event, so nothing depends on the network:
 * - `networkMode: "always"`: the library's default, `"online"`, pauses every query and action
 *   while the system says it is offline (the `offline` event, which a closed laptop lid, a VPN
 *   change or a flaky Wi-Fi sends), and the panel would stop reading and acting;
 * - `refetchOnReconnect: false` and `refetchOnWindowFocus: false`: the engine's events keep the
 *   data current (useEngineEvents); reading everything again when the network comes back or the
 *   window gets focus would only re-run the environment check and the Linear calls (one cheap
 *   local read turns the focus read back on for itself: `useCliPath`);
 * - `retry: false`: a local call that failed fails the same way again, and retrying behind the
 *   user's back would only show the error state seconds later.
 */
export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { networkMode: "always", retry: false, refetchOnReconnect: false, refetchOnWindowFocus: false },
      mutations: { networkMode: "always", retry: false },
    },
  });
}
