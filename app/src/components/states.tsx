import { CircleAlert, LoaderCircle, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { cleanDisplayText } from "@/lib/clean";
import { describeError } from "@/lib/errors";

/** Nothing to show yet, and what to do about it: a centred icon, a title, a sentence and maybe a button. */
export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
}: {
  icon: LucideIcon;
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center gap-3 px-6 py-12 text-center">
      <Icon aria-hidden className="size-10 text-muted-foreground" />
      <p className="text-base font-medium">{title}</p>
      <p className="max-w-md text-muted-foreground">{description}</p>
      {action}
    </div>
  );
}

/** Placeholder rows while data is being read. */
export function LoadingState({ rows = 3 }: { rows?: number }) {
  const { t } = useTranslation();
  return (
    <div role="status" aria-label={t("common.loading")} className="flex flex-col gap-3">
      {Array.from({ length: rows }, (_, row) => (
        <Skeleton key={row} className="h-10 w-full" />
      ))}
    </div>
  );
}

/**
 * The first connection to Linear is still under way (`LinearSetup::Connecting`): what needs Linear
 * waits for it. Until then a call that asks Linear is refused ("still connecting"), which is no
 * failure to report; the panel reads it again once the connection changes.
 */
export function LinearConnecting() {
  const { t } = useTranslation();
  return (
    <p role="status" className="flex items-center gap-2 text-muted-foreground">
      <LoaderCircle aria-hidden className="size-4 shrink-0 animate-spin" />
      {t("linear.connecting")}
    </p>
  );
}

/**
 * One line of the engine's or an executor's own words, cleaned of escape sequences and control
 * characters, in a secondary style. It always sits below a translated headline and is shown as
 * text, never interpreted; selectable so that it can be copied into a bug report.
 */
export function DetailLine({ text }: { text: string }) {
  const clean = cleanDisplayText(text).trim();
  if (clean === "") return null;
  return <p className="font-mono text-xs break-words text-muted-foreground select-text">{clean}</p>;
}

/**
 * A failed read or action: the translated headline (`headline` is a translation key, by default
 * "Something went wrong"), the backend's line as a `DetailLine`, and a retry when there is one.
 */
export function ErrorState({ error, headline, onRetry }: { error: unknown; headline?: string; onRetry?: () => void }) {
  const { t } = useTranslation();
  const described = describeError(error, t, headline ?? "common.error_title");
  return (
    <Alert variant="destructive">
      <CircleAlert aria-hidden />
      <AlertTitle>{described.headline}</AlertTitle>
      <AlertDescription>
        {described.detail !== null && <DetailLine text={described.detail} />}
        {onRetry && (
          <Button variant="outline" size="sm" className="mt-2 text-foreground" onClick={onRetry}>
            {t("common.retry")}
          </Button>
        )}
      </AlertDescription>
    </Alert>
  );
}
