import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { vi } from "vitest";
import { initI18nOnce, setLanguage } from "@/i18n";
import en from "@/i18n/locales/en.json";
import zhCN from "@/i18n/locales/zh-CN.json";
import { ErrorBoundary } from "./ErrorBoundary";

/** A component that fails while it renders, as a page does when it meets data it cannot read. */
function Bomb({ message }: { message: string }): never {
  throw new Error(message);
}

beforeEach(async () => {
  await initI18nOnce("en");
  // React reports an error it hands to a boundary; keep the output of the test run readable.
  vi.spyOn(console, "error").mockImplementation(() => {});
});

afterEach(async () => {
  await setLanguage("en");
});

test("what renders is shown as it is", () => {
  render(
    <ErrorBoundary>
      <p>the panel</p>
    </ErrorBoundary>,
  );
  expect(screen.getByText("the panel")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: en.crash.reload })).not.toBeInTheDocument();
});

test("a render error shows a translated error page instead of a blank window", () => {
  render(
    <ErrorBoundary>
      <Bomb message={"cannot read the run\u001b[2J"} />
    </ErrorBoundary>,
  );
  expect(screen.getByRole("heading", { name: en.crash.title })).toBeInTheDocument();
  expect(screen.getByText(en.crash.hint)).toBeInTheDocument();
  // The error's own words are a detail, cleaned, under the translated headline.
  expect(screen.getByText("cannot read the run")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: en.crash.reload })).toBeInTheDocument();
});

test("the reload button reloads through the handler it was given", async () => {
  const onReload = vi.fn();
  render(
    <ErrorBoundary onReload={onReload}>
      <Bomb message="boom" />
    </ErrorBoundary>,
  );
  await userEvent.click(screen.getByRole("button", { name: en.crash.reload }));
  expect(onReload).toHaveBeenCalledTimes(1);
});

test("the error is logged, with where it happened", () => {
  const log = vi.mocked(console.error);
  render(
    <ErrorBoundary>
      <Bomb message="boom" />
    </ErrorBoundary>,
  );
  const ours = log.mock.calls.find(([text]) => typeof text === "string" && text.includes("panel stopped"));
  expect(ours).toBeDefined();
  expect(ours![1]).toBeInstanceOf(Error);
  expect((ours![1] as Error).message).toBe("boom");
  // The component stack names the component that failed.
  expect(String(ours![2])).toContain("Bomb");
});

test("an error after the first render is caught too", async () => {
  function Later() {
    const [broken, setBroken] = useState(false);
    if (broken) throw new Error("it broke later");
    return (
      <button type="button" onClick={() => setBroken(true)}>
        break
      </button>
    );
  }
  render(
    <ErrorBoundary>
      <Later />
    </ErrorBoundary>,
  );
  await userEvent.click(screen.getByRole("button", { name: "break" }));
  expect(screen.getByRole("heading", { name: en.crash.title })).toBeInTheDocument();
  expect(screen.getByText("it broke later")).toBeInTheDocument();
});

test("the page is in the interface language", async () => {
  await initI18nOnce("zh-CN");
  render(
    <ErrorBoundary>
      <Bomb message="boom" />
    </ErrorBoundary>,
  );
  expect(screen.getByRole("heading", { name: zhCN.crash.title })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: zhCN.crash.reload })).toBeInTheDocument();
});

test("an error without any words is a headline and a button, nothing else of its own", () => {
  function Silent(): never {
    throw new Error("");
  }
  render(
    <ErrorBoundary>
      <Silent />
    </ErrorBoundary>,
  );
  expect(screen.getByRole("heading", { name: en.crash.title })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: en.crash.reload })).toBeInTheDocument();
});

test("something thrown that is not an Error is shown too", () => {
  function Odd(): never {
    // A library may throw anything.
    throw "a plain string";
  }
  render(
    <ErrorBoundary>
      <Odd />
    </ErrorBoundary>,
  );
  expect(screen.getByText("a plain string")).toBeInTheDocument();
});
