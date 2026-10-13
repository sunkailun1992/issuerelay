import { useEffect, useState } from "react";

/**
 * Renders the component again every `periodMs`, so that what it shows by the clock (how long a
 * run has been going, "5 minutes ago") keeps up. The timer ends with the component.
 */
export function useTick(periodMs: number): void {
  const [, setTicks] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => setTicks((ticks) => ticks + 1), periodMs);
    return () => clearInterval(timer);
  }, [periodMs]);
}
