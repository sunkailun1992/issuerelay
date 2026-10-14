/**
 * The steps of the first-run guide, in order (design 8). Each name is also the step's key in the
 * translations: `onboarding.step.*` (the list), `onboarding.title.*` (the heading).
 */
export const STEPS = ["language", "linear", "workspace", "executors", "binding", "finish"] as const;

export type StepId = (typeof STEPS)[number];

/** What a step gets from the guide around it. */
export interface StepNav {
  /** Where the step stands in the guide, counting from 0. */
  index: number;
  /** The guide is saving its end: nothing may take the user off the step meanwhile. */
  locked: boolean;
  onBack: () => void;
  onNext: () => void;
}
