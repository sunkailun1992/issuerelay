import { expect, test } from "vitest";
import cases from "./routing-cases.json";
import { bindingIdFor, bindingsOverlap, uniqueBindingId } from "./routing";

// The tables are shared: app/src-tauri/src/commands.rs reads the same file and checks it against
// core's `config::bindings_overlap` and `config::binding_id_for`, so the binding form and the engine
// cannot disagree about which bindings take the same issues or about a new binding's id.
test.each(cases.overlap)("$a and $b overlap: $overlap", ({ a, b, overlap }) => {
  expect(bindingsOverlap({ project_id: "proj-1", labels: a }, { project_id: "proj-1", labels: b })).toBe(overlap);
  expect(bindingsOverlap({ project_id: "proj-1", labels: b }, { project_id: "proj-1", labels: a })).toBe(overlap);
});

test.each(cases.ids)("$project with $labels is $id", ({ project, labels, id }) => {
  expect(bindingIdFor(project, labels)).toBe(id);
});

test("bindings of different projects never overlap, whatever their labels", () => {
  expect(bindingsOverlap({ project_id: "proj-1" }, { project_id: "proj-2" })).toBe(false);
  expect(bindingsOverlap({ project_id: "proj-1", labels: ["Docs"] }, { project_id: "proj-2", labels: ["Docs"] })).toBe(false);
});

test("a binding without the labels field is the project's default binding", () => {
  expect(bindingsOverlap({ project_id: "proj-1" }, { project_id: "proj-1", labels: [] })).toBe(true);
  expect(bindingsOverlap({ project_id: "proj-1" }, { project_id: "proj-1", labels: ["Docs"] })).toBe(false);
});

test("a new binding's id that a binding has already gets the first number after it that none has", () => {
  expect(uniqueBindingId("proj-1#Docs", ["proj-1", "proj-2"])).toBe("proj-1#Docs");
  expect(uniqueBindingId("proj-1#Docs", ["proj-1", "proj-1#Docs"])).toBe("proj-1#Docs-2");
  expect(uniqueBindingId("proj-1#Docs", ["proj-1#Docs", "proj-1#Docs-2", "proj-1#Docs-4"])).toBe("proj-1#Docs-3");
  expect(uniqueBindingId("proj-3", ["proj-3"])).toBe("proj-3-2");
});
