/**
 * An async subscription inside `useEffect`: `useEffect(() => subscription(backend.onResync(f)), [...])`.
 * The returned cleanup also unsubscribes when the component unmounts before the subscription
 * resolved.
 */
export function subscription(start: Promise<() => void>): () => void {
  let stop: (() => void) | undefined;
  let ended = false;
  start.then(
    (unsubscribe) => {
      if (ended) unsubscribe();
      else stop = unsubscribe;
    },
    // Without a listener nothing in the panel would refresh itself: say so where a developer looks.
    (error: unknown) => console.error("could not subscribe to the backend's events", error),
  );
  return () => {
    ended = true;
    stop?.();
  };
}
