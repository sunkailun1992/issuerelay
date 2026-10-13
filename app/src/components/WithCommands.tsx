/** Characters that mean something in a regular expression, made plain. */
const plain = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/**
 * A sentence that holds commands, with each command set as code. The translation gives the words
 * and the code gives the commands (`t("…", { command })`), so the commands are found in the
 * translated text by what they are, in whichever language it is in; a text that does not hold one
 * is shown whole. The longest command is looked for first, so that one that holds another is not
 * cut in two. A command is one piece on a line: it moves to the next line whole and is never
 * broken in the middle.
 */
export function WithCommands({ text, commands }: { text: string; commands: readonly string[] }) {
  const wanted = commands.filter((command) => command !== "");
  if (wanted.length === 0) return <>{text}</>;
  const pattern = new RegExp(`(${[...wanted].sort((a, b) => b.length - a.length).map(plain).join("|")})`);
  // A capture group keeps what it found: the commands are the odd parts.
  return (
    <>
      {text.split(pattern).map((part, index) =>
        index % 2 === 1 ? (
          <code key={index} className="rounded bg-muted px-1 py-0.5 font-mono text-xs whitespace-nowrap text-foreground select-text">
            {part}
          </code>
        ) : (
          part
        ),
      )}
    </>
  );
}
