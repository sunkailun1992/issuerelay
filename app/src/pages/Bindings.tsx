import { useTranslation } from "react-i18next";
import { PageHeader } from "@/components/PageHeader";

export default function Bindings() {
  const { t } = useTranslation();
  return <PageHeader title={t("nav.bindings")} />;
}
