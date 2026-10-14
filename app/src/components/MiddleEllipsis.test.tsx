import { render, screen } from "@testing-library/react";
import { MiddleEllipsis } from "./MiddleEllipsis";

test("a short text is one element with the whole text", () => {
  render(<MiddleEllipsis text="~/code/demo-app" />);
  const text = screen.getByText("~/code/demo-app");
  expect(text).toHaveAttribute("title", "~/code/demo-app");
});

const LONG = "~/code/an-extremely-long-folder-name/with/several/levels/demo-app";

test("a long text keeps its start and its last 24 characters, so that the end is never lost", () => {
  const { container } = render(<MiddleEllipsis text={LONG} />);
  const visible = [...container.querySelectorAll("[aria-hidden=true]")].map((part) => part.textContent);
  expect(visible).toEqual([LONG.slice(0, -24), LONG.slice(-24)]);
  expect(visible.join("")).toBe(LONG);
});

test("the whole of a long text is there for screen readers, for copying and on hover", () => {
  const { container } = render(<MiddleEllipsis text={LONG} />);
  // One element has it all; the two visible halves are hidden from assistive technology.
  expect(screen.getByText(LONG)).toHaveClass("sr-only");
  expect(container.firstElementChild).toHaveAttribute("title", LONG);
});

test("only the first half gives way when the room is short", () => {
  const { container } = render(<MiddleEllipsis text={LONG} />);
  const [head, tail] = [...container.querySelectorAll("[aria-hidden=true]")];
  expect(head).toHaveClass("overflow-hidden", "text-ellipsis");
  expect(tail).toHaveClass("shrink-0");
});

test.each([
  // The cut falls right after a blank, or right before one.
  { where: "after", text: `~/projects/${"a".repeat(20)} ${"b".repeat(24)}` },
  { where: "before", text: `~/projects/${"a".repeat(20)} ${"b".repeat(23)}` },
])("a blank where the text is cut ($where) stays, so that the halves read as the text does", ({ where, text }) => {
  const { container } = render(<MiddleEllipsis text={text} />);
  const [head, tail] = [...container.querySelectorAll("[aria-hidden=true]")];
  expect(where === "after" ? head.textContent : tail.textContent).toMatch(where === "after" ? / $/ : /^ /);
  expect(head.textContent + tail.textContent).toBe(text);
  // White space is kept as it is: it would be dropped at the end of a line otherwise.
  expect(head).toHaveClass("whitespace-pre");
  expect(tail).toHaveClass("whitespace-pre");
});

test("characters outside the basic plane are not cut in two", () => {
  const folder = String.fromCodePoint(0x1f4c1);
  const text = `~/code/${folder.repeat(30)}`;
  const { container } = render(<MiddleEllipsis text={text} />);
  const visible = [...container.querySelectorAll("[aria-hidden=true]")].map((part) => part.textContent);
  // 24 characters, each a pair of UTF-16 units.
  expect([...(visible[1] ?? "")]).toHaveLength(24);
  expect(visible.join("")).toBe(text);
});
