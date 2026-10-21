import type { TFunction } from "i18next";
import type { Binding, BindingReport, Effort, ExecutorKind, LeanStatus, PermissionProfile, Settings } from "./types";

/**
 * The names of the executors, permission levels and reasoning efforts as a binding shows them. A
 * binding that names none follows the settings, and says so with the one it follows: "Default
 * (Claude Code)"; settings without a default effort leave it to the executor. Until
 * the settings are read (or when they cannot be) the default is not guessed: it reads "Unknown".
 * The settings are given, not read here: the caller has them already, and a query that failed
 * would be tried again by every component that asked for it anew.
 */
export function bindingLabels(t: TFunction, settings: Settings | undefined) {
  const unknown = t("common.unknown");
  return {
    executor: (kind?: ExecutorKind): string =>
      kind === undefined
        ? t("executor.default", { name: settings === undefined ? unknown : t(`executor.${settings.default_executor}`) })
        : t(`executor.${kind}`),
    permission: (profile?: PermissionProfile): string =>
      profile === undefined
        ? t("permission.default", { name: settings === undefined ? unknown : t(`permission.${settings.default_permission}`) })
        : t(`permission.${profile}`),
    effort: (level?: Effort): string => {
      if (level !== undefined) return t(`effort.${level}`);
      const name =
        settings === undefined
          ? unknown
          : settings.default_effort === undefined
            ? t("effort.executor")
            : t(`effort.${settings.default_effort}`);
      return t("effort.default", { name });
    },
    /**
     * Whether `binding` runs lean: what the last check `found`, else what the binding and the
     * settings say. Only the check knows whether its folder is the home folder; without the
     * settings a binding that follows them may be one with personal settings, so it is unknown.
     */
    lean: (binding: Binding, found?: LeanStatus): string => {
      if (found !== undefined) return t(`bindings.lean_state.${found}`);
      if (binding.lean === false) return t("bindings.lean_state.off");
      const permission = binding.permission ?? settings?.default_permission;
      if (permission === undefined) return unknown;
      return t(`bindings.lean_state.${permission === "personal" ? "personal" : "on"}`);
    },
  };
}

/** The labels `bindingLabels` makes. */
export type BindingLabels = ReturnType<typeof bindingLabels>;

/**
 * Whether the executor of `report` can take the binding's runs, as one sentence: a binding held
 * back only by the lean checks says so, as its issues do when they wait.
 */
export function executorVerdict(t: TFunction, report: BindingReport): string {
  const executor = t(`executor.${report.executor}`);
  if (report.executor_ready) return t("bindings.executor_ready", { executor });
  return t(report.lean_not_ready ? "bindings.executor_lean_not_ready" : "bindings.executor_not_ready", { executor });
}
