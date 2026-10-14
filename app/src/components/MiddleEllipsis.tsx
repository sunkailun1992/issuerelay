import { cn } from "@/lib/utils";

/** How many characters at the end stay in view when the room is short: a folder's own name and a bit of its parents. */
const TAIL = 24;

/**
 * A text on one line that gives way in the middle when there is not room for all of it, so that
 * both ends of a long folder path stay readable: where it starts and, above all, which folder it
 * is. The whole text is the hover text and, for screen readers and for copying, one element of
 * its own; the two halves that are seen are hidden from assistive technology (they would read as
 * two texts). Characters are counted whole, so that a character made of two UTF-16 units is not
 * cut in half; white space is kept where the cut is, or the halves would not read as the text does.
 */
export function MiddleEllipsis({ text, className }: { text: string; className?: string }) {
  const characters = [...text];
  if (characters.length <= TAIL) {
    return (
      <span title={text} className={cn("block truncate", className)}>
        {text}
      </span>
    );
  }
  return (
    <span title={text} className={cn("flex min-w-0", className)}>
      <span className="sr-only">{text}</span>
      <span aria-hidden="true" className="overflow-hidden text-ellipsis whitespace-pre">
        {characters.slice(0, -TAIL).join("")}
      </span>
      <span aria-hidden="true" className="shrink-0 whitespace-pre">
        {characters.slice(-TAIL).join("")}
      </span>
    </span>
  );
}
