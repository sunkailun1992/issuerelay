import defaults from "@/lib/trigger-defaults.json";
import type { TriggerNames } from "@/lib/types";
import { useSettings } from "./queries";

/**
 * The names of the trigger in a default configuration, which the panel shows until its settings
 * have been read (the first-run gate has read them before any page shows). The only copy is the
 * table trigger-defaults.json, which the app crate's tests hold against `TriggerNames::default()`.
 */
const DEFAULT_NAMES: TriggerNames = defaults;

/** The label and the statuses that trigger a run, as the settings name them. */
export function useTriggerNames(): TriggerNames {
  return useSettings().data?.trigger ?? DEFAULT_NAMES;
}
