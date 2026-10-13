import type { TriggerNames } from "@/lib/types";
import { useSettings } from "./queries";

/**
 * The names of the trigger in a default configuration (`config::Settings::default()`). Shown only
 * until the settings have been read, which the first-run gate has done before any page shows.
 */
const DEFAULT_NAMES: TriggerNames = {
  label: "ai-ready",
  todo: "Todo",
  in_progress: "In Progress",
  in_testing: "In Testing",
  done: "Done",
};

/** The label and the statuses that trigger a run, as the settings name them. */
export function useTriggerNames(): TriggerNames {
  return useSettings().data?.trigger ?? DEFAULT_NAMES;
}
