import { CircleCheck, LoaderCircle, LogIn, LogOut, RefreshCw, TriangleAlert, Unplug, type LucideIcon } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { useAppInfo, useLinearLogin, useLinearLoginWaiting, useLinearLogout, useLinearReconnect, useStatus } from "@/hooks/queries";
import { useLang } from "@/hooks/useLang";
import { useTick } from "@/hooks/useTick";
import { formatRelative } from "@/lib/format";
import { cn } from "@/lib/utils";
import { DetailLine, ErrorState, LoadingState } from "./states";

/** How often "5 minutes ago" is worked out again. */
const CLOCK_MS = 30_000;

/** How long the first connection may take before the card says what it may be waiting for. */
const SLOW_CONNECTION_MS = 30_000;

/**
 * Under a first connection that has gone on for `SLOW_CONNECTION_MS` on macOS: a new build's first
 * read of the saved login waits while the keychain asks whether that build may use it, and the
 * dialog can be behind other windows (v0.2.3 acceptance P1; core's log says so after a minute,
 * `noting_a_slow_attempt` in `host.rs`). The status region is there from the start and filled in
 * later, so that screen readers announce the line when it appears.
 */
function KeychainHint() {
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    const timer = setTimeout(() => setSlow(true), SLOW_CONNECTION_MS);
    return () => clearTimeout(timer);
  }, []);
  return (
    <p role="status" className="text-xs text-muted-foreground">
      {slow && <KeychainHintText />}
    </p>
  );
}

/**
 * The hint's words, on macOS only; the platform is asked for once they are due. Windows has no
 * such dialog to point at: the login is read with `CredReadW`, which shows no prompt
 * (windows-native-keyring-store 1.1.0, read 2026-10-22).
 */
function KeychainHintText() {
  const { t } = useTranslation();
  const info = useAppInfo();
  return info.data?.platform === "macos" ? t("linear.keychain_hint") : null;
}

/** The look of a connection: a round icon in the colour of how it stands. */
type Tone = "good" | "bad" | "neutral" | "busy";
const TONE: Record<Tone, string> = {
  good: "bg-state-done/10 text-state-done",
  bad: "bg-state-failed/10 text-state-failed",
  neutral: "bg-muted text-muted-foreground",
  busy: "bg-state-running/10 text-state-running",
};

/** One card: the icon, what is the matter (the headline, then the lines under it), and the buttons. */
function Layout({ icon: Icon, tone, spin, children, actions }: { icon: LucideIcon; tone: Tone; spin?: boolean; children: ReactNode; actions?: ReactNode }) {
  return (
    <Card size="sm">
      {/* A row of its own: the card's content slot is a column. */}
      <div className="flex flex-wrap items-center justify-between gap-3 px-(--card-spacing)">
        <div className="flex min-w-0 items-center gap-3">
          <span className={cn("flex size-9 shrink-0 items-center justify-center rounded-full", TONE[tone])}>
            <Icon aria-hidden className={cn("size-5", spin && "animate-spin")} />
          </span>
          <div className="flex min-w-0 flex-col gap-0.5">{children}</div>
        </div>
        {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
      </div>
    </Card>
  );
}

/**
 * The connection to Linear (design 7): how it stands and what can be done about it. Every
 * headline comes from structured data (`StatusView.linear`, the login that is under way); the
 * engine's own sentence, if there is one, is the detail line under it. `manage` is for the
 * environment check, where a connection (a working one, or one that failed) can also be
 * authorized again or ended. `labels` is for a page that words the first connection its own way
 * (the first-run guide): the text of the login button, and of the connection being under way.
 */
export function LinearStatus({
  manage = false,
  labels,
}: {
  manage?: boolean;
  labels?: { login?: string; connecting?: string };
}) {
  const { t } = useTranslation();
  const lang = useLang();
  const status = useStatus();
  const login = useLinearLogin();
  const logout = useLinearLogout();
  const reconnect = useLinearReconnect();
  // Any login that waits for the browser, wherever it was started: core does one change of the
  // login at a time, so the card shows it and nothing else can be started meanwhile.
  const waiting = useLinearLoginWaiting();
  // "Last check 5 minutes ago" stays right while the page is open.
  useTick(CLOCK_MS);

  if (status.isError && !status.isFetching) {
    return <ErrorState error={status.error} headline="overview.status_failed" onRetry={() => void status.refetch()} />;
  }
  if (status.data === undefined) return <LoadingState rows={1} />;
  const { status: snapshot, linear } = status.data;

  // What the environment check adds to a connection that exists but is not what it should be: a
  // new authorization (the stored token may have been revoked, which trying again cannot mend)
  // and logging out. The waiting state above is the same whichever card started the login.
  const managing = manage && (
    <>
      <Button variant="outline" size="sm" onClick={() => login.mutate()}>
        <LogIn aria-hidden />
        {t("linear.log_in_again")}
      </Button>
      <Button variant="outline" size="sm" disabled={logout.isPending} onClick={() => logout.mutate()}>
        <LogOut aria-hidden />
        {t("linear.log_out")}
      </Button>
    </>
  );

  if (waiting) {
    return (
      <Layout
        icon={LoaderCircle}
        tone="busy"
        spin
        actions={
          <Button variant="outline" size="sm" onClick={() => void login.cancel()}>
            {t("linear.cancel_login")}
          </Button>
        }
      >
        <p className="font-medium">{t("linear.waiting_browser")}</p>
      </Layout>
    );
  }

  switch (linear.kind) {
    case "connecting":
      return (
        <Layout icon={LoaderCircle} tone="busy" spin>
          <p className="font-medium">{labels?.connecting ?? t("linear.connecting")}</p>
          <KeychainHint />
        </Layout>
      );
    case "not_configured":
      return (
        <Layout
          icon={Unplug}
          tone="neutral"
          actions={
            <Button size="sm" onClick={() => login.mutate()}>
              <LogIn aria-hidden />
              {labels?.login ?? t("linear.log_in")}
            </Button>
          }
        >
          <p className="font-medium">{t("linear.not_configured")}</p>
        </Layout>
      );
    case "connect_failed":
      return (
        <Layout
          icon={TriangleAlert}
          tone="bad"
          actions={
            <>
              <Button variant="outline" size="sm" disabled={reconnect.isPending} onClick={() => reconnect.mutate()}>
                <RefreshCw aria-hidden className={cn(reconnect.isPending && "animate-spin")} />
                {t("linear.retry")}
              </Button>
              {managing}
            </>
          }
        >
          <p className="font-medium">{t("linear.connect_failed")}</p>
          <DetailLine text={linear.message} />
        </Layout>
      );
    case "interface_changed":
      return (
        <Layout icon={TriangleAlert} tone="bad">
          <p className="font-medium">{t("linear.interface_changed")}</p>
          <DetailLine text={linear.message} />
        </Layout>
      );
    case "connected":
      return (
        <Layout
          icon={CircleCheck}
          tone="good"
          actions={managing}
        >
          {snapshot.viewer === null ? (
            <p className="font-medium">{t("linear.connected_plain")}</p>
          ) : (
            <p className="font-medium">
              {t("linear.connected")} <span className="font-semibold">{snapshot.viewer.name}</span>
            </p>
          )}
          <p className="text-xs text-muted-foreground">
            {snapshot.last_poll_at === null
              ? t("linear.never_checked")
              : t("linear.last_check", { time: formatRelative(snapshot.last_poll_at, lang) })}
          </p>
        </Layout>
      );
  }
}
