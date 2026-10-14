import type { TFunction } from "i18next";
import { toast } from "sonner";
import { BackendError } from "./backend";
import { cleanDisplayText } from "./clean";
import type { UpdateFailureCode } from "./types";
import updateFailures from "./update-failure-cases.json";

/**
 * A failure as the panel shows it (design 9.3: no bare error text): a translated headline, and
 * the backend's own line, cleaned, as a secondary detail. An error the backend answers with as a
 * code gets that code's text as the headline and no detail.
 */
export function describeError(error: unknown, t: TFunction, headlineKey: string): { headline: string; detail: string | null } {
  if (error instanceof BackendError && error.code !== null) {
    return { headline: t(`errors.${error.code}`), detail: null };
  }
  const detail = cleanDisplayText(error instanceof Error ? error.message : String(error)).trim();
  return { headline: t(headlineKey), detail: detail === "" ? null : detail };
}

/** Tells the user that something failed, as a toast: `describeError`'s headline and detail, both plain text. */
export function reportFailure(error: unknown, t: TFunction, headlineKey: string): void {
  const { headline, detail } = describeError(error, t, headlineKey);
  toast.error(headline, { description: detail ?? undefined });
}

/** The translation key of an update failure's headline: `errors.<error code>`, by update-failure-cases.json. */
export function updateFailureKey(code: UpdateFailureCode): string {
  const entry = updateFailures.find((candidate) => candidate.code === code);
  return `errors.${entry?.error_code ?? "update_failed"}`;
}
