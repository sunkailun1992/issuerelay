import type { ReactNode } from "react";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";

/**
 * A labelled field: the label, then the control and what is said about it. `htmlFor` names the
 * control; a field whose place holds a message instead (the list that could not be read) has
 * none, since a label cannot name what is not there.
 */
export function Field({ label, htmlFor, children }: { label: string; htmlFor?: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={htmlFor}>{label}</Label>
      {children}
    </div>
  );
}

/** A line under a control; `id` is what the control's `aria-describedby` names. */
export function Hint({ id, tone = "muted", children }: { id: string; tone?: "muted" | "error"; children: ReactNode }) {
  return (
    <p id={id} className={cn("text-xs", tone === "error" ? "text-destructive" : "text-muted-foreground")}>
      {children}
    </p>
  );
}
