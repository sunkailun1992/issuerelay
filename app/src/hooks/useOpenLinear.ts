import { useTranslation } from "react-i18next";
import { useBackend } from "@/lib/backend-context";
import { reportFailure } from "@/lib/errors";

/**
 * Opens a Linear link in the browser. The link goes to the backend as it is: only the app decides
 * what is a Linear link and opens it, and says so (as a code the panel translates) when it is
 * not; the panel opens nothing itself. A failure is reported to the user as a failure.
 */
export function useOpenLinear(): (url: string) => void {
  const { t } = useTranslation();
  const backend = useBackend();
  return (url) => {
    backend.openLinear(url).catch((error: unknown) => reportFailure(error, t, "actions.failed.open_linear"));
  };
}
