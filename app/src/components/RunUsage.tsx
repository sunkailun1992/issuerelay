import { useTranslation } from "react-i18next";
import { useLang } from "@/hooks/useLang";
import { formatCost, formatTokens } from "@/lib/format";
import type { ExecutorKind, RunUsage } from "@/lib/types";

/**
 * What a run used, as its executor reported it (run-improvements design, section 7.3): the cost
 * (Claude only, an estimate), the tokens in and out, the cache and the turns, each when reported,
 * with a note under them on what the figures are.
 */
export function RunUsageSummary({ usage, executor }: { usage: RunUsage; executor: ExecutorKind | null }) {
  const { t } = useTranslation();
  const lang = useLang();
  const tokens = (count: number) => formatTokens(count, lang);
  const parts: string[] = [];
  if (usage.cost_micro_usd !== null) parts.push(t("runs.usage.cost", { cost: formatCost(usage.cost_micro_usd) }));
  if (usage.input_tokens !== null || usage.output_tokens !== null) {
    parts.push(t("runs.usage.tokens", { input: tokens(usage.input_tokens ?? 0), output: tokens(usage.output_tokens ?? 0) }));
  }
  // Without a count of what was written to the cache, only what was read from it.
  if (usage.cache_write_tokens !== null) {
    parts.push(t("runs.usage.cache", { read: tokens(usage.cache_read_tokens ?? 0), write: tokens(usage.cache_write_tokens) }));
  } else if (usage.cache_read_tokens !== null) {
    parts.push(t("runs.usage.cache_read", { read: tokens(usage.cache_read_tokens) }));
  }
  if (usage.turns !== null) parts.push(t("runs.usage.turns", { count: usage.turns }));
  const note = executor === "codex" ? t("runs.usage.codex_note") : usage.cost_micro_usd !== null ? t("runs.usage.cost_note") : null;
  return (
    <span className="flex flex-col gap-0.5">
      <span>{parts.join(" · ")}</span>
      {note !== null && <span className="text-xs text-muted-foreground">{note}</span>}
    </span>
  );
}
