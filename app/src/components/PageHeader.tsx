import type { ReactNode, Ref } from "react";

/**
 * The top of every page: its title, a sentence about it and the page's own actions on the right.
 * The heading can be given the focus by script (not by Tab), for a page that has to put the focus
 * somewhere when what had it is gone: it takes its element with `headingRef`.
 */
export function PageHeader({
  title,
  description,
  actions,
  headingRef,
}: {
  title: string;
  description?: ReactNode;
  actions?: ReactNode;
  headingRef?: Ref<HTMLHeadingElement>;
}) {
  return (
    <header className="mb-6 flex items-start justify-between gap-4">
      <div className="flex min-w-0 flex-col gap-1">
        {/* Not interactive: it takes no ring when the focus is put on it, and the screen reader reads it out. */}
        <h1 ref={headingRef} tabIndex={-1} className="text-xl font-semibold outline-none">
          {title}
        </h1>
        {description ? <p className="text-muted-foreground">{description}</p> : null}
      </div>
      {actions ? <div className="flex shrink-0 items-center gap-2">{actions}</div> : null}
    </header>
  );
}
