import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardFooter, CardHeader } from "@/components/ui/card";
import { useBackend } from "@/lib/backend-context";
import { reportFailure } from "@/lib/errors";
import type { StartupCode } from "@/lib/types";
import { DetailLine } from "./states";

/**
 * The page that replaces the panel when the host could not start: what happened (a translated
 * headline for the kind of failure, the engine's own line cleaned below it) and the one thing
 * left to do, quit. Every failure code has its page; nothing here depends on the engine running.
 */
export function StartupError({ code, detail }: { code: StartupCode; detail: string }) {
  const backend = useBackend();
  const { t } = useTranslation();
  return (
    <main className="flex min-h-screen items-center justify-center bg-background p-(--page-padding)">
      <Card className="w-full max-w-lg">
        <CardHeader>
          <h1 className="font-heading text-base leading-normal font-medium">{t("startup.failed_title")}</h1>
        </CardHeader>
        <CardContent>
          <p>{t(`startup.${code}`)}</p>
          <DetailLine text={detail} />
        </CardContent>
        <CardFooter>
          <Button onClick={() => backend.requestQuit().catch((error: unknown) => reportFailure(error, t, "common.error_title"))}>
            {t("startup.quit")}
          </Button>
        </CardFooter>
      </Card>
    </main>
  );
}
