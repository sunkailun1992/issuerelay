import { useEffect } from "react";

const DARK = "(prefers-color-scheme: dark)";

/**
 * Keeps the `dark` class of `<html>` in step with the system's appearance, also while the panel
 * is open. The colours of both themes are in styles.css.
 */
export function useSystemTheme(): void {
  useEffect(() => {
    const dark = window.matchMedia(DARK);
    const apply = () => document.documentElement.classList.toggle("dark", dark.matches);
    apply();
    dark.addEventListener("change", apply);
    return () => dark.removeEventListener("change", apply);
  }, []);
}
