import { Copy } from "lucide-react";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { ErrorState, LoadingState } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Dialog, DialogClose, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger,
} from "@/components/ui/dialog";
import { useCodexSessionPreview } from "@/hooks/queries";
import { useCopyText } from "@/hooks/useCopyText";
import { cleanDisplayLines } from "@/lib/clean";
import { isActive } from "@/lib/runs";
import type { Job } from "@/lib/types";

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

function CodexSessionDialog({ id, sessionId }: { id: number; sessionId: string }) {
  const { t } = useTranslation();
  const preview = useCodexSessionPreview(id);
  const data = preview.data;
  return (
    <DialogContent className="max-h-[calc(100dvh-2rem)] grid-rows-[auto_minmax(0,1fr)_auto] sm:max-w-2xl" showCloseButton={false}>
      <DialogHeader>
        <DialogTitle>{t("provider_session.preview")}</DialogTitle>
        <DialogDescription>{t("provider_session.preview_description")}</DialogDescription>
      </DialogHeader>
      <div className="-m-1 min-h-0 space-y-3 overflow-y-auto p-1">
        {preview.isPending || preview.isFetching ? (
          <LoadingState />
        ) : preview.isError ? (
          <ErrorState error={preview.error} headline="provider_session.read_failed" onRetry={() => void preview.refetch()} />
        ) : data?.session_id !== sessionId ? (
          <p role="alert">{t("provider_session.session_mismatch")}</p>
        ) : (
          <>
            {data.title && <p className="wrap-anywhere text-muted-foreground">{cleanDisplayLines(data.title)}</p>}
            {data.messages.length === 0 && <p>{t("provider_session.empty")}</p>}
            {data.messages.map(({ role, text }, index) => (
              <div className="rounded-lg border p-3" key={index}>
                <h3 className="mb-1 font-medium">{t(`provider_session.role.${role}`)}</h3>
                <p className="wrap-anywhere whitespace-pre-wrap select-text">{cleanDisplayLines(text)}</p>
              </div>
            ))}
            {data.has_more && <p className="text-muted-foreground">{t("provider_session.more")}</p>}
          </>
        )}
      </div>
      <DialogFooter>
        <DialogClose asChild><Button variant="outline">{t("common.close")}</Button></DialogClose>
      </DialogFooter>
    </DialogContent>
  );
}

/** Copies references to the original session without starting or attaching to it. */
export function ProviderSessionCard({ job }: { job: Job }) {
  const { t } = useTranslation();
  const copy = useCopyText();
  const heading = useId();
  const [open, setOpen] = useState(false);
  const { directory, executor, session_id: sessionId } = job;
  const running = job.state === "running" && job.started_at !== null;
  const finished = !isActive(job) && job.finished_at !== null;
  if ((!running && !finished) || !directory || (executor !== "codex" && executor !== "claude") ||
      sessionId == null || sessionId.length !== 36 || !UUID.test(sessionId)) return null;
  const codex = executor === "codex";
  const verification = job.mode === "verify";
  const command = codex ? `codex resume ${sessionId}` : `claude --resume ${sessionId}`;
  const desktop = codex ? `codex://threads/${sessionId}` : `claude --desktop --resume ${sessionId}`;
  const claudeHint = verification ? "claude_verification" : "claude_desktop";
  const desktopHint = codex ? "codex_desktop_hint" : `${claudeHint}${running ? "_running" : ""}_hint`;
  return (
    <section aria-labelledby={heading}>
      <Card size="sm">
        <CardContent className="space-y-3">
          <h2 id={heading} className="font-medium">{t("provider_session.title")}</h2>
          {finished && (
            <div className="space-y-2">
              <p className="text-sm text-muted-foreground">{t(verification
                ? "provider_session.verification_reopen_hint" : "provider_session.reopen_hint")}</p>
              <code className="block font-mono text-xs wrap-anywhere select-text">{command}</code>
              <div className="flex flex-wrap gap-2">
                <Button variant="outline" size="sm" onClick={() => copy(command)}>
                  <Copy aria-hidden />
                  {t("provider_session.copy")}
                </Button>
                {codex && (
                  <Dialog open={open} onOpenChange={setOpen}>
                    <DialogTrigger asChild><Button variant="outline" size="sm">{t("provider_session.preview")}</Button></DialogTrigger>
                    {open && <CodexSessionDialog id={job.id} sessionId={sessionId} />}
                  </Dialog>
                )}
              </div>
              {codex && <p className="text-xs text-muted-foreground">{t("provider_session.archive_hint")}</p>}
            </div>
          )}
          <div className="space-y-2">
            <p className="text-sm text-muted-foreground">{t(`provider_session.${desktopHint}`)}</p>
            <code className="block font-mono text-xs wrap-anywhere select-text">{desktop}</code>
            <Button variant="outline" size="sm" onClick={() => copy(desktop)}>
              <Copy aria-hidden />
              {t(codex ? "provider_session.copy_codex_desktop" : "provider_session.copy_claude_desktop")}
            </Button>
          </div>
        </CardContent>
      </Card>
    </section>
  );
}
