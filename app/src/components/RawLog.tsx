import { Copy } from "lucide-react";
import { useLayoutEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useCopyText } from "@/hooks/useCopyText";
import { cleanDisplayText } from "@/lib/clean";

/** How far from the end of the log, in pixels, the reader may be and still count as following it. */
const FOLLOW_SLACK_PX = 8;

/**
 * A run's log as it is, one line of the file for each line shown, each cleaned (`cleanDisplayText`),
 * in a box that scrolls. "Copy all" copies the lines as shown. While the run is going (`live`) the
 * box keeps to the end of the log as it grows; once the reader scrolls up it stays where they are,
 * and it follows again when they scroll back to the end.
 */
export function RawLog({ lines, live }: { lines: string[]; live: boolean }) {
  const { t } = useTranslation();
  const copy = useCopyText();
  const text = useMemo(() => lines.map(cleanDisplayText).join("\n"), [lines]);
  const frame = useRef<HTMLDivElement>(null);
  const following = useRef(true);

  useLayoutEffect(() => {
    const viewport = frame.current?.querySelector<HTMLElement>("[data-slot=scroll-area-viewport]");
    if (!viewport) return;
    const onScroll = () => {
      following.current = viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight <= FOLLOW_SLACK_PX;
    };
    viewport.addEventListener("scroll", onScroll);
    return () => viewport.removeEventListener("scroll", onScroll);
  }, []);

  useLayoutEffect(() => {
    const viewport = frame.current?.querySelector<HTMLElement>("[data-slot=scroll-area-viewport]");
    if (live && following.current && viewport) viewport.scrollTop = viewport.scrollHeight;
  }, [text, live]);

  return (
    <div className="flex flex-col gap-2">
      <div className="flex justify-end">
        <Button variant="outline" size="sm" onClick={() => copy(text)}>
          <Copy aria-hidden />
          {t("runs.copy_all")}
        </Button>
      </div>
      <div ref={frame}>
        {/* The box is as tall as the log up to 60% of the window, then it scrolls. */}
        <ScrollArea className="rounded-lg border bg-card [&_[data-slot=scroll-area-viewport]]:max-h-[60vh]">
          <pre className="p-3 font-mono text-xs wrap-anywhere whitespace-pre-wrap select-text">{text}</pre>
        </ScrollArea>
      </div>
    </div>
  );
}
