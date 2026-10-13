import { expect, test } from "vitest";
import i18n from "@/i18n";
import { BackendError } from "./backend";
import { describeError } from "./errors";

const t = i18n.t.bind(i18n);

test("a failure is a translated headline with the backend's line, cleaned, as the detail", () => {
  expect(describeError(new BackendError("config.toml: unknown key `foo`\u001b[0m"), t, "actions.failed.save_settings")).toEqual({
    headline: "Could not save the settings",
    detail: "config.toml: unknown key `foo`",
  });
});

test("an error that is a code is translated and has no detail", () => {
  expect(describeError(new BackendError("not_a_linear_link"), t, "actions.failed.pause")).toEqual({
    headline: "IssueRelay only opens Linear's own pages",
    detail: null,
  });
  expect(describeError(new BackendError("login_cancelled"), t, "actions.failed.linear_login")).toEqual({
    headline: "The browser login was cancelled",
    detail: null,
  });
});

test("anything that is not a backend error still gets a headline", () => {
  expect(describeError(new Error("boom"), t, "common.error_title")).toEqual({ headline: "Something went wrong", detail: "boom" });
  expect(describeError("plain text", t, "common.error_title")).toEqual({ headline: "Something went wrong", detail: "plain text" });
  expect(describeError(undefined, t, "common.error_title").headline).toBe("Something went wrong");
});

test("a message with nothing left after cleaning has no detail", () => {
  expect(describeError(new Error("\u001b[2J  "), t, "common.error_title").detail).toBeNull();
});

test("the headline follows the language it is asked in", async () => {
  const zh = i18n.getFixedT("zh-CN");
  expect(describeError(new Error("boom"), zh, "actions.failed.pause").headline).toBe("无法暂停监控");
});
