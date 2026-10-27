import { sameName } from "./settings-form";
import type { Binding } from "./types";

/** What decides which issues a binding takes. */
type Routed = Pick<Binding, "project_id" | "labels">;

/**
 * Whether two bindings would take the same issues, as core's `config::bindings_overlap`: they are
 * of the same project and are both its default binding (no routing labels), or share a routing
 * label. Saving a binding replaces the ones it overlaps (`Engine::add_binding`).
 */
export function bindingsOverlap(a: Routed, b: Routed): boolean {
  if (a.project_id !== b.project_id) return false;
  const ours = a.labels ?? [];
  const theirs = b.labels ?? [];
  if (ours.length === 0 || theirs.length === 0) return ours.length === 0 && theirs.length === 0;
  return ours.some((label) => theirs.some((other) => sameName(label, other)));
}

/**
 * The id a new binding gets, as core's `config::binding_id_for`: the project's id for its default
 * binding, the project's id and the first routing label after `#` for the others.
 */
export function bindingIdFor(projectId: string, labels: string[]): string {
  return labels.length === 0 ? projectId : `${projectId}#${labels[0].trim()}`;
}

/**
 * `id`, or when a binding has it already, the first of `id-2`, `id-3`, … that none has (run-improvements
 * design, 9.3): a binding keeps its id when its labels change, so the id a new one would get can be taken.
 */
export function uniqueBindingId(id: string, taken: readonly string[]): string {
  if (!taken.includes(id)) return id;
  let n = 2;
  while (taken.includes(`${id}-${n}`)) n++;
  return `${id}-${n}`;
}
