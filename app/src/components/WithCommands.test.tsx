import { render, screen } from "@testing-library/react";
import { WithCommands } from "./WithCommands";

test("a command in a sentence is set as code, and the words around it are left as they are", () => {
  render(
    <p>
      <WithCommands text="Run claude in a terminal and log in" commands={["claude"]} />
    </p>,
  );
  const command = screen.getByText("claude");
  expect(command.tagName).toBe("CODE");
  expect(command.parentElement).toHaveTextContent("Run claude in a terminal and log in");
});

test("every command is set as code, once each time it occurs", () => {
  render(
    <p>
      <WithCommands text="Run codex mcp list to find it, then codex mcp login <name>. codex mcp list again." commands={["codex mcp list", "codex mcp login <name>"]} />
    </p>,
  );
  expect(screen.getAllByText("codex mcp list")).toHaveLength(2);
  expect(screen.getByText("codex mcp login <name>").tagName).toBe("CODE");
});

test("a command that holds another is not cut in two", () => {
  render(
    <p>
      <WithCommands text="Use codex login, not codex" commands={["codex", "codex login"]} />
    </p>,
  );
  expect(screen.getByText("codex login").tagName).toBe("CODE");
  expect(screen.getByText("codex").tagName).toBe("CODE");
});

test("a sentence that does not hold the command (another language may word it differently) is shown whole", () => {
  render(
    <p>
      <WithCommands text="Anmelden im Terminal" commands={["claude"]} />
    </p>,
  );
  expect(screen.getByText("Anmelden im Terminal")).toBeInTheDocument();
  expect(document.querySelector("code")).toBeNull();
});

test("characters that mean something in a pattern are only text", () => {
  render(
    <p>
      <WithCommands text="Run a.b* (now) [x]" commands={["a.b*", "(now)", "[x]"]} />
    </p>,
  );
  expect(screen.getByText("a.b*").tagName).toBe("CODE");
  expect(screen.getByText("(now)").tagName).toBe("CODE");
  expect(screen.getByText("[x]").tagName).toBe("CODE");
});

test("no commands leave the text alone", () => {
  render(
    <p>
      <WithCommands text="Nothing to set" commands={[]} />
    </p>,
  );
  expect(screen.getByText("Nothing to set")).toBeInTheDocument();
});
