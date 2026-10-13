import { screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { createMockBackend } from "@/lib/mock";
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
  expect(screen.getByText("ai-ready/Todo/Done")).toBeInTheDocument();
  expect(await screen.findByText("bot-ready/Ready/Shipped")).toBeInTheDocument();
});

test("settings that cannot be read leave the default names", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.settings = async () => {
    throw new Error("cannot read the settings");
  };
  await renderWithProviders(<Names />, { backend });
  await new Promise((done) => setTimeout(done, 10));
  expect(screen.getByText("ai-ready/Todo/Done")).toBeInTheDocument();
});
