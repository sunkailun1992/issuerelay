import { expect, test } from "vitest";
import cases from "./model-name-cases.json";
import { isValidModelName } from "./model-name";

// The table is shared: app/src-tauri/src/commands.rs reads the same file and checks it against
// core's `config::is_valid_model_name`, so the panel and the engine cannot disagree about a name.
// Its first rows are the plain cases (a normal name, each mark that is allowed, the length limit,
// and what is refused); the later ones are the edges: the one mark that may not start a name and
// the ones that may, a letter outside ASCII, and a name that ends in a line break.
test.each(cases)("$name: valid $valid", ({ name, valid }) => {
  expect(isValidModelName(name)).toBe(valid);
});
