/**
 * A model name goes to the executor as one argument, so core accepts 1 to 100 ASCII letters,
 * digits, `.`, `_`, `:` and `-`, and refuses a name that starts with `-` (it would read as a
 * switch): `config::is_valid_model_name`. Names with brackets (`sonnet[1m]`) are refused too.
 * This is the same rule so that the panel can say no before anything is saved;
 * model-name-cases.json is the table both sides are checked against.
 */
export function isValidModelName(text: string): boolean {
  // `$` without the `m` flag matches only at the very end, so a name ending in a line break fails.
  return /^[A-Za-z0-9._:][A-Za-z0-9._:-]{0,99}$/.test(text);
}
