import { screen, waitFor } from "@testing-library/react";
import { expect, test } from "vitest";
import { queryKeys } from "@/hooks/queries";
import { createMockBackend } from "@/lib/mock";
import { DEFAULT_SETTINGS } from "@/lib/mock-fixtures";
import defaults from "@/lib/trigger-defaults.json";
import { renderWithProviders } from "@/test/render";
import { useTriggerNames } from "./useTriggerNames";

function Names() {
  const names = useTriggerNames();
  return (
    <p>
      {names.label}/{names.todo}/{names.done}
    </p>
  );
}

test("the names are the default ones until the settings are read, then the configured ones", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const settings = await backend.settings();
  backend.settings = async () => ({ ...settings, trigger: { ...settings.trigger, label: "bot-ready", todo: "Ready", done: "Shipped" } });
  await renderWithProviders(<Names />, { backend });
  expect(screen.getByText(`${defaults.label}/${defaults.todo}/${defaults.done}`)).toBeInTheDocument();
  expect(await screen.findByText("bot-ready/Ready/Shipped")).toBeInTheDocument();
});

test("the default names are the table that a Rust test holds against core's defaults", () => {
  // trigger-defaults.json is the only copy in the panel; the app crate's tests compare it with
  // `TriggerNames::default()`. The mock's default settings take their names from it too.
  expect(Object.keys(defaults).sort()).toEqual(["done", "in_progress", "in_testing", "label", "plan_label", "todo"]);
  expect(DEFAULT_SETTINGS.trigger).toEqual(defaults);
});

test("settings that cannot be read leave the default names", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.settings = async () => {
    throw new Error("cannot read the settings");
  };
  const { client } = await renderWithProviders(<Names />, { backend });
  // Until the read has failed there is nothing to prove: it has, once the cache says so.
  await waitFor(() => expect(client.getQueryState(queryKeys.settings)?.status).toBe("error"));
  expect(screen.getByText(`${defaults.label}/${defaults.todo}/${defaults.done}`)).toBeInTheDocument();
});
