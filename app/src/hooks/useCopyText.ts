import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { reportFailure } from "@/lib/errors";

/**
 * Copies text to the system clipboard and tells the user how it went: "Copied", or the failure
 * ("Could not copy" with the reason). A window that has no clipboard at all fails the same way.
 */
export function useCopyText(): (text: string) => void {
  const { t } = useTranslation();
  return (text) => {
    // `async` makes a missing clipboard (an error thrown at once) a failed copy like any other.
    void (async () => navigator.clipboard.writeText(text))().then(
      () => toast.success(t("common.copied")),
      (error: unknown) => reportFailure(error, t, "actions.failed.copy"),
    );
  };
}
