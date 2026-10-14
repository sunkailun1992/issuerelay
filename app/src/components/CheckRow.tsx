import { CircleCheck, CircleX } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * One line of a check, for a list: a symbol and what is found. The symbol is never the only sign
 * (it is hidden from a screen reader): the text says whether the check passed.
 */
export function CheckRow({ ok, children }: { ok: boolean; children: ReactNode }) {
  const Icon = ok ? CircleCheck : CircleX;
  return (
    <li className="flex items-start gap-1.5">
      <Icon aria-hidden className={cn("mt-0.5 size-4 shrink-0", ok ? "text-state-done" : "text-state-failed")} />
      <div className="flex min-w-0 flex-col gap-0.5">{children}</div>
    </li>
  );
}
