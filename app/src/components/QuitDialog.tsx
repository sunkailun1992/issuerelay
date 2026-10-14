import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { subscription } from "@/hooks/subscription";
import { useBackend } from "@/lib/backend-context";
import { reportFailure } from "@/lib/errors";
import type { QuitMode } from "@/lib/types";

/**
 * The question that quitting with runs in progress asks (design 4.1, 6.2): wait for them, stop
 * them now, or keep running. Waiting can still become stopping, so once chosen the prompt stays
 * up with "stop now" until the app exits. "Keep running" only closes the prompt: the app treats
 * a question nobody answers like no question at all and asks again the next time. When quitting
 * already waits (`QuitPrompt.waiting`: a panel reloaded after "wait" has lost the prompt, and the
 * app tells it again at the next Quit), the prompt opens on the waiting state with "stop now".
 */
export function QuitDialog() {
  const backend = useBackend();
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  // Kept after the prompt closes, so that it does not read "0 runs" while it fades out.
  const [running, setRunning] = useState(0);
  const [waiting, setWaiting] = useState(false);
  const [stopping, setStopping] = useState(false);

  useEffect(
    () =>
      subscription(
        backend.onQuitRequested((prompt) => {
          setRunning(prompt.running);
          setWaiting(prompt.waiting);
          setStopping(false);
          setOpen(true);
        }),
      ),
    [backend],
  );

  const answer = (mode: QuitMode) => {
    const mark = mode === "wait" ? setWaiting : setStopping;
    mark(true);
    backend.quit(mode).catch((error: unknown) => {
      // The app did not take the answer: back to the question, and say why.
      mark(false);
      reportFailure(error, t, "common.error_title");
    });
  };

  return (
    <AlertDialog
      open={open}
      // Escape and a click outside answer "keep running", but only while the question is open.
      onOpenChange={(next) => {
        if (!next && !waiting && !stopping) setOpen(false);
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("quit.title")}</AlertDialogTitle>
          <AlertDialogDescription>{waiting ? t("quit.waiting") : t("quit.body", { count: running })}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          {!waiting && <AlertDialogCancel disabled={stopping}>{t("quit.cancel")}</AlertDialogCancel>}
          <Button variant="destructive" disabled={stopping} onClick={() => answer("stop")}>
            {t("quit.stop")}
          </Button>
          {!waiting && (
            <Button disabled={stopping} onClick={() => answer("wait")}>
              {t("quit.wait")}
            </Button>
          )}
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
