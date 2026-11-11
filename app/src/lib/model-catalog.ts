import type { ExecutorKind } from "./types";

export type ModelCatalogSource = "native" | "cache" | "builtin";

/** Matches core's `model_catalog::ModelOption`. Effort names come from the model's catalog. */
export interface ModelOption {
  id: string;
  label: string;
  efforts: string[];
  default_effort: string | null;
  recommended: boolean;
  is_default: boolean;
}

export interface ModelCatalog {
  executor: ExecutorKind;
  source: ModelCatalogSource;
  models: ModelOption[];
}

/** Keep a configured model selectable even when the refreshed catalog omits it. */
export function catalogModels(
  catalog: ModelCatalog | undefined,
  executor: ExecutorKind,
  model?: string,
): readonly ModelOption[] {
  const models = catalog?.executor === executor ? catalog.models : [];
  if (!model || models.some(({ id }) => id === model)) return models;
  return [...models, {
    id: model,
    label: model,
    efforts: [],
    default_effort: null,
    recommended: false,
    is_default: false,
  }];
}

/** Offer Ultra only when a Codex model advertises it. Keep an existing selection visible. */
export function modelEffortOptions(
  catalog: ModelCatalog | undefined,
  executor: ExecutorKind,
  model: string | undefined,
  current: string | undefined,
  fallback: readonly string[],
): readonly string[] {
  const selected = catalog?.executor === executor ? catalog.models.find(({ id }) => id === model) : undefined;
  const choices = selected?.efforts ?? fallback.filter((effort) => effort !== "ultra");
  return current && !choices.includes(current) ? [...choices, current] : choices;
}
