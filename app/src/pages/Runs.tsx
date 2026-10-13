import { useTranslation } from "react-i18next";
import { PageHeader } from "@/components/PageHeader";

export default function Runs() {
  const { t } = useTranslation();
  return <PageHeader title={t("nav.runs")} />;
}
