import { useTranslation } from "react-i18next";
import { useOpenLinear } from "@/hooks/useOpenLinear";

/**
 * An issue's identifier that opens the issue in Linear. It is a button, since it does something
 * (the backend opens the browser), in the look of a link. It takes the place of a separate "Open
 * in Linear" button, so that a row of a table spends no room on one: what is read is the
 * identifier, the name for screen readers and the tooltip say what the click does.
 */
export function IssueLink({ identifier, url }: { identifier: string; url: string }) {
  const { t } = useTranslation();
  const open = useOpenLinear();
  return (
    <button
      type="button"
      title={t("actions.open_in_linear")}
      aria-label={t("actions.open_issue_in_linear", { identifier })}
      onClick={() => open(url)}
      className="rounded-sm font-mono text-xs text-primary underline-offset-4 outline-none hover:underline focus-visible:ring-3 focus-visible:ring-ring/50"
    >
      {identifier}
    </button>
  );
}
