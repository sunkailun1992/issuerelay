import type { TFunction } from "i18next";
import type { ExecutorKind, PermissionProfile, Settings } from "./types";

/**
 * The names of the executors and permission levels as a binding shows them. A binding that names
 * none follows the settings, and says so with the one it follows: "Default (Claude Code)". Until
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
  };
}

/** The labels `bindingLabels` makes. */
export type BindingLabels = ReturnType<typeof bindingLabels>;
