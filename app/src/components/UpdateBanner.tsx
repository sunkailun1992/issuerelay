import { CircleAlert, Download } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { DetailLine } from "@/components/states";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { useInstallUpdate, useUpdateStatus } from "@/hooks/queries";
import { updateFailureKey } from "@/lib/errors";

/**
 * Above every page (design 12): a newer version with "Install and restart", or, once after a
 * restart, an update that could not be installed and why. Installing asks the Rust side, which
 * downloads and verifies the update and then goes through the quit flow: the panel itself never
 * quits or restarts. "Later" hides an offer for that version and "Dismiss" the failure, until the
 * panel is reloaded or the app restarts: closing the panel only hides its window, and the page
 * keeps what it remembers. A failed check is left to the settings page: monitoring goes on, so it
 * does not nag here.
 *
 * Wrapping: the alert is a grid whose text column would grow to the width of the longest word,
 * so the title and the text have `min-w-0` and may break inside a word (a version number, a file
 * name in the reason of a failure); the two buttons wrap under each other when the page is too
 * narrow for both.
 */
export function UpdateBanner() {
  const { t } = useTranslation();
  const status = useUpdateStatus();
  const install = useInstallUpdate();
  const [laterFor, setLaterFor] = useState<string | null>(null);
  const [failureSeen, setFailureSeen] = useState(false);
  const shown = status.data;
  if (shown?.kind === "available" && laterFor !== shown.version) {
    return (
      <Alert className="mb-4" role="status">
        <Download className="size-4" aria-hidden />
        <AlertTitle className="min-w-0 wrap-anywhere">{t("update.available", { version: shown.version })}</AlertTitle>
        <AlertDescription className="min-w-0 space-y-2">
          <p className="text-muted-foreground">{t("update.restart_note")}</p>
          <div className="flex flex-wrap gap-2">
            <Button size="sm" disabled={install.isPending} onClick={() => install.mutate()}>
              {install.isPending ? t("update.downloading") : t("update.install")}
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setLaterFor(shown.version)}>
              {t("update.later")}
            </Button>
          </div>
        </AlertDescription>
      </Alert>
    );
  }
  if (shown?.kind === "failed" && shown.code === "install_failed" && !failureSeen) {
    return (
      <Alert className="mb-4" role="alert">
        <CircleAlert className="size-4" aria-hidden />
        <AlertTitle className="min-w-0 wrap-anywhere">{t(updateFailureKey(shown.code))}</AlertTitle>
        <AlertDescription className="min-w-0 space-y-2">
          <DetailLine text={shown.detail} />
          <Button size="sm" variant="outline" onClick={() => setFailureSeen(true)}>
            {t("update.dismiss")}
          </Button>
        </AlertDescription>
      </Alert>
    );
  }
  return null;
}
