import { expect, test } from "vitest";

import { catalogModels, modelEffortOptions, type ModelCatalog } from "./model-catalog";

const catalog: ModelCatalog = {
  executor: "codex",
  source: "native",
  models: [
    {
      id: "gpt-6.1-sol", label: "GPT-6.1 Sol", efforts: ["low", "max", "ultra", "future-mode"],
      default_effort: "low", recommended: true, is_default: true,
    },
    {
      id: "gpt-6-luna", label: "GPT-6 Luna", efforts: ["low", "max"],
      default_effort: "low", recommended: false, is_default: false,
    },
  ],
};

test("refresh keeps a model no longer listed in the catalog", () => {
  const options = catalogModels(catalog, "codex", "custom.model");
  expect(options.map(({ id }) => id)).toEqual(["gpt-6.1-sol", "gpt-6-luna", "custom.model"]);
  expect(catalogModels(catalog, "claude", "opus").map(({ id }) => id)).toEqual(["opus"]);
});

test("effort choices use the selected model's metadata and retain a saved effort", () => {
  const fallback = ["low", "medium", "max", "ultra"];
  expect(modelEffortOptions(catalog, "codex", "gpt-6.1-sol", undefined, fallback))
    .toEqual(["low", "max", "ultra", "future-mode"]);
  expect(modelEffortOptions(catalog, "codex", "gpt-6-luna", undefined, fallback))
    .toEqual(["low", "max"]);
  expect(modelEffortOptions(catalog, "codex", "gpt-6-luna", "ultra", fallback))
    .toEqual(["low", "max", "ultra"]);
  expect(modelEffortOptions(catalog, "claude", "opus", undefined, fallback))
    .toEqual(["low", "medium", "max"]);
});
