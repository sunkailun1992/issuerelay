import { useTranslation } from "react-i18next";
import { PageHeader } from "@/components/PageHeader";

export default function Issues() {
  const { t } = useTranslation();
  return <PageHeader title={t("nav.issues")} />;
}
