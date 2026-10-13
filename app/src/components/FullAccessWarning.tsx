import { TriangleAlert } from "lucide-react";
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

/**
 * What the first choice of full access means (design 14): the AI can run any command on this
 * computer with all of the user's credentials, and the rules in the prompt and the repository's
 * AGENTS.md are its only limits; a lower level is the way out. The caller decides what understanding
 * it does (the binding form saves the confirmation in the settings, `full_access_acknowledged`,
 * so that it is asked once) and what saying no does: `onCancel` is also what Escape does.
 */
export function FullAccessWarning({ open, onConfirm, onCancel }: { open: boolean; onConfirm: () => void; onCancel: () => void }) {
  const { t } = useTranslation();
  return (
    <AlertDialog
      open={open}
      // What closes it from inside is only "no" (the cancel button, Escape): the answer "I understand"
      // is a plain button, whose click would otherwise close the dialog and count as a cancel too.
      onOpenChange={(next) => {
        if (!next) onCancel();
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle className="flex items-center gap-2">
            <TriangleAlert aria-hidden className="size-5 shrink-0 text-state-failed" />
            {t("bindings.full_access_title")}
          </AlertDialogTitle>
          <AlertDialogDescription>{t("bindings.full_access_body")}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          {/* The focus starts here: Enter does not confirm by accident. */}
          <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
          <Button onClick={onConfirm}>{t("bindings.full_access_ok")}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
