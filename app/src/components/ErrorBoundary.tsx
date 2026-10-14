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
  /**
   * What stands in for what failed: the error page (by default), or nothing, for a part of the
   * panel that must not take the rest with it (the quit prompt, the toasts).
   */
  fallback?: "page" | "nothing";
  /**
   * A change of it clears a failure and renders the children again: the boundary around the
   * pages is given the address, so going to another page recovers from one that failed.
   */
  resetKey?: string;
  /** How the Reload button reloads, once the address is back at the overview; the page's own reload by default. */
  onReload?: () => void;
}

/** `failure` holds what was thrown, wrapped: anything can be thrown, including `undefined`. */
interface State {
  failure: { error: unknown } | null;
  /** The reset key the failure belongs to. */
  resetKey: string | undefined;
}

/**
 * A render error anywhere below shows the crash page instead of leaving the window blank (React
 * unmounts the whole tree of an error nothing catches), or nothing (`fallback`), and is logged.
 * Errors in event handlers and in promises are not render errors; they are reported where they
 * happen. Reload first takes the address back to the overview: a page that fails on its data
 * would fail again at its own address.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { failure: null, resetKey: this.props.resetKey };

  static getDerivedStateFromError(error: unknown): Partial<State> {
    return { failure: { error } };
  }

  static getDerivedStateFromProps(props: Props, state: State): Partial<State> | null {
    return props.resetKey === state.resetKey ? null : { failure: null, resetKey: props.resetKey };
  }

  componentDidCatch(error: unknown, info: ErrorInfo): void {
    // The webview's console is the only log the panel has; the app's own log cannot be reached from here.
    console.error(
      this.props.fallback === "nothing" ? "a part of the panel stopped on an error" : "the panel stopped on an error",
      error,
      info.componentStack,
    );
  }

  private reload = (): void => {
    // Only the address within the panel changes; the page's own address stays as it is.
    window.history.replaceState(null, "", "#/");
    (this.props.onReload ?? reloadPage)();
  };

  render(): ReactNode {
    if (this.state.failure === null) return this.props.children;
    if (this.props.fallback === "nothing") return null;
    return <CrashPage error={this.state.failure.error} onReload={this.reload} />;
  }
}
