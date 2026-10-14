import { useTranslation } from "react-i18next";
import type { Lang } from "@/i18n";

/** The interface language, which `lib/format.ts` takes to word a time or a duration. */
export function useLang(): Lang {
  return useTranslation().i18n.language as Lang;
}
