import { screen } from "@testing-library/react";
import { JOB_STATES } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import { PLANNED_STYLE, StatusBadge, STATE_STYLE } from "./StatusBadge";

test("every state has its own label, colour and icon", async () => {
  await renderWithProviders(<>{JOB_STATES.map((s) => <StatusBadge key={s} state={s} />)}</>, { lang: "zh-CN" });
  for (const label of ["等待绑定", "排队中", "运行中", "待验收", "待补充信息", "已完成", "失败", "已取消"]) {
    expect(screen.getByText(label)).toBeInTheDocument();
  }
  const classes = new Set(JOB_STATES.map((s) => STATE_STYLE[s].className));
  const icons = new Set(JOB_STATES.map((s) => STATE_STYLE[s].icon));
  expect(classes.size).toBe(JOB_STATES.length);
  expect(icons.size).toBe(JOB_STATES.length);
});

test("each badge is coloured by its own state variable: text, a 30% border and a 10% tint", () => {
  for (const state of JOB_STATES) {
    const [text, border, tint] = STATE_STYLE[state].className.split(" ");
    const name = text.replace("text-state-", "");
    expect(name, state).not.toBe(text);
    expect(border, state).toBe(`border-state-${name}/30`);
    expect(tint, state).toBe(`bg-state-${name}/10`);
  }
});

test("only a running job's icon spins", async () => {
  const { container } = await renderWithProviders(<>{JOB_STATES.map((s) => <StatusBadge key={s} state={s} />)}</>);
  const spinning = container.querySelectorAll(".animate-spin");
  expect(spinning).toHaveLength(1);
  expect(spinning[0].closest("[data-slot=badge]")).toHaveTextContent("Running");
});

test("the badge reads in the interface language", async () => {
  await renderWithProviders(<StatusBadge state="awaiting_acceptance" />, { lang: "ja" });
  expect(screen.getByText("確認待ち")).toBeInTheDocument();
});

test("a posted plan has its own name and icon, in the colour of a run that waits for the user", async () => {
  const { container } = await renderWithProviders(<StatusBadge state="needs_info" planned />, { lang: "zh-TW" });
  expect(screen.getByText("方案已出")).toBeInTheDocument();
  expect(PLANNED_STYLE.className).toBe(STATE_STYLE.needs_info.className);
  expect(new Set(JOB_STATES.map((s) => STATE_STYLE[s].icon)).has(PLANNED_STYLE.icon)).toBe(false);
  expect(container.querySelector("[data-slot=badge]")).toHaveClass(...STATE_STYLE.needs_info.className.split(" "));
});

test("only a run that waits for the user can show a posted plan", async () => {
  await renderWithProviders(<StatusBadge state="failed" planned />);
  expect(screen.getByText("Failed")).toBeInTheDocument();
  expect(screen.queryByText("Plan posted")).not.toBeInTheDocument();
});
