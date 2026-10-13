import { render, screen } from "@testing-library/react";
import App from "./App";

test("the panel renders its title", () => {
  render(<App />);
  expect(screen.getByRole("heading", { name: "IssueRelay" })).toBeInTheDocument();
});
