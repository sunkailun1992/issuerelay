import { useTranslation } from "react-i18next";
import { PageHeader } from "@/components/PageHeader";

export default function Onboarding() {
  const { t } = useTranslation();
  return (
    <main className="mx-auto w-full max-w-(--content-max-width) p-(--page-padding)">
      <PageHeader title={t("app.name")} description={t("app.tagline")} />
    </main>
  );
}
