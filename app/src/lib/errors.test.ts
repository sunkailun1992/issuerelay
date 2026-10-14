import { expect, test } from "vitest";
import i18n from "@/i18n";
import { BackendError, ERROR_CODES } from "./backend";
import { describeError, updateFailureKey } from "./errors";
import { UPDATE_FAILURE_CODES } from "./types";
import cases from "./update-failure-cases.json";

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

test("update failures follow the case table the Rust side reads too", () => {
  expect(cases.map((entry) => entry.code)).toEqual([...UPDATE_FAILURE_CODES]);
  for (const { code, error_code } of cases) {
    expect(ERROR_CODES).toContain(error_code);
    expect(updateFailureKey(code as (typeof UPDATE_FAILURE_CODES)[number])).toBe(`errors.${error_code}`);
    // `install_update` answers with the error code, which reads as the same translated headline.
    expect(describeError(new BackendError(error_code), t, "actions.failed.install_update")).toEqual({
      headline: t(`errors.${error_code}`),
      detail: null,
    });
  }
});

test("every error code is in the case table or is one of the other two", () => {
  const updateCodes = cases.map((entry) => entry.error_code);
  const others = ERROR_CODES.filter((code) => !updateCodes.includes(code));
  expect(others).toEqual(["not_a_linear_link", "login_cancelled"]);
});

test("a cancelled browser login is still a code, not a failure", () => {
  expect(new BackendError("login_cancelled").code).toBe("login_cancelled");
});

test("update failures read in every language, each with its own words", async () => {
  for (const lang of ["zh-CN", "zh-TW", "ja", "ko"]) {
    const other = i18n.getFixedT(lang);
    for (const { error_code } of cases) {
      expect(other(`errors.${error_code}`), `${lang}: ${error_code}`).not.toBe(t(`errors.${error_code}`));
    }
  }
});
