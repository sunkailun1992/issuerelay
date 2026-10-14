// The ANSI escape sequences the panel removes: CSI (ESC [ or the one-byte U+009B, parameters,
// one final byte), OSC ended by BEL or ST (ESC \), and the two-character ESC sequences.
// eslint-disable-next-line no-control-regex -- matching terminal escape sequences is the point.
const ANSI = /(?:\u001b\[|\u009b)[0-?]*[ -/]*[@-~]|\u001b\][^\u0007\u001b]*(?:\u0007|\u001b\\)|\u001b[^[\]]/g;

/** The characters core's `text::is_invisible` removes; keep in step with crates/core/src/text.rs. */
function isInvisible(code: number): boolean {
  return (
    code === 0x00ad ||
    code === 0x061c ||
    code === 0x115f ||
    code === 0x1160 ||
    code === 0x200b ||
    code === 0x200e ||
    code === 0x200f ||
    (code >= 0x2028 && code <= 0x202e) ||
    (code >= 0x2060 && code <= 0x206f) ||
    code === 0x3164 ||
    code === 0xfeff ||
    code === 0xffa0 ||
    (code >= 0xe0000 && code <= 0xe007f)
  );
}

/** Rust's `char::is_control`: the C0 controls, DEL and the C1 controls. */
function isControl(code: number): boolean {
  return code <= 0x1f || (code >= 0x7f && code <= 0x9f);
}

/**
 * Text from executors, the AI or the engine as the panel shows it: escape sequences removed,
 * every other control character turned into a space, invisible characters dropped. Unlike core's
 * `clean_text` it neither trims nor shortens, so a log line keeps its indentation. The cases in
 * clean-cases.json are checked here and, for the character rules, against core's `clean_text`
 * in commands.rs (which trims what it returns).
 */
export function cleanDisplayText(text: string): string {
  let out = "";
  for (const char of text.replace(ANSI, "")) {
    const code = char.codePointAt(0) ?? 0;
    if (isControl(code)) out += " ";
    else if (!isInvisible(code)) out += char;
  }
  return out;
}

/** `cleanDisplayText` for each line of a multi-line text (command output, tool results). */
export function cleanDisplayLines(text: string): string {
  return text.split(/\r?\n/).map(cleanDisplayText).join("\n");
}
