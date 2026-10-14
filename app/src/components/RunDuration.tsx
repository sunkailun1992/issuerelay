import { useLang } from "@/hooks/useLang";
import { useTick } from "@/hooks/useTick";
import { formatDuration } from "@/lib/format";
import type { Job } from "@/lib/types";

/** How long a run that is going has been going: counted again every second. */
function RunningTime({ since }: { since: string }) {
  const lang = useLang();
  useTick(1000);
  return formatDuration(since, null, lang);
}

/**
 * How long a run took: counted on while it runs, from its start to its end once it has ended, and
 * a dash for a run that has not started (or has no end recorded).
 */
export function RunDuration({ job }: { job: Job }) {
  const lang = useLang();
  if (job.started_at === null) return "—";
  if (job.state === "running") return <RunningTime since={job.started_at} />;
  return job.finished_at === null ? "—" : formatDuration(job.started_at, job.finished_at, lang);
}
