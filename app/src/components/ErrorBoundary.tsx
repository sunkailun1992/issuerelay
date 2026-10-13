import { Component, type ErrorInfo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardFooter, CardHeader } from "@/components/ui/card";
import { DetailLine } from "./states";

function reloadPage(): void {
  window.location.reload();
}

/**
 * What stands in for a panel that stopped on a render error: a translated headline, a sentence
 * on what to do, the error's own words as a cleaned detail, and the one thing to do. It needs
 * nothing of the panel (no backend, no data), because it is shown when the panel is what failed.
 */
function CrashPage({ error, onReload }: { error: unknown; onReload: () => void }) {
  const { t } = useTranslation();
  return (
    <main className="flex min-h-screen items-center justify-center bg-background p-(--page-padding)">
      <Card className="w-full max-w-lg">
        <CardHeader>
          <h1 className="font-heading text-base leading-normal font-medium">{t("crash.title")}</h1>
        </CardHeader>
        <CardContent>
          <p>{t("crash.hint")}</p>
          <DetailLine text={error instanceof Error ? error.message : String(error)} />
        </CardContent>
        <CardFooter>
          <Button onClick={onReload}>{t("crash.reload")}</Button>
        </CardFooter>
      </Card>
    </main>
  );
}

interface Props {
  children: ReactNode;
  /** What the Reload button does; it reloads the page by default. */
  onReload?: () => void;
}

/** `failure` holds what was thrown, wrapped: anything can be thrown, including `undefined`. */
interface State {
  failure: { error: unknown } | null;
}

/**
 * A render error anywhere below shows the crash page instead of leaving the window blank (React
 * unmounts the whole tree of an error nothing catches), and is logged. Errors in event handlers
 * and in promises are not render errors; they are reported where they happen.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { failure: null };

  static getDerivedStateFromError(error: unknown): State {
    return { failure: { error } };
  }

  componentDidCatch(error: unknown, info: ErrorInfo): void {
    // The webview's console is the only log the panel has; the app's own log cannot be reached from here.
    console.error("the panel stopped on an error", error, info.componentStack);
  }

  render(): ReactNode {
    if (this.state.failure === null) return this.props.children;
    return <CrashPage error={this.state.failure.error} onReload={this.props.onReload ?? reloadPage} />;
  }
}
