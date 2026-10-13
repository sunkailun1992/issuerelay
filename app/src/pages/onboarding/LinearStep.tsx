import { ChevronDown } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ApiKeyForm } from "@/components/ApiKeyForm";
import { LinearStatus } from "@/components/LinearStatus";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { useLinearLoginWaiting, useStatus } from "@/hooks/queries";
import { StepFrame } from "./StepFrame";
import type { StepNav } from "./steps";

/**
 * The second step: the connection to Linear. The card is the one of the other pages, worded for
 * the first connection: while it is under way (a stored login being checked) there is nothing to
 * press, and with no login the browser login is offered, which waits for the browser and can be
 * given up. A personal API key is the way for a connection that is not there or did not work; it
 * waits while a browser login does, because core changes the login one at a time. The guide goes
 * on once Linear is connected.
 */
export function LinearStep({ nav }: { nav: StepNav }) {
  const { t } = useTranslation();
  const status = useStatus();
  const waiting = useLinearLoginWaiting();
  const [keyOpen, setKeyOpen] = useState(false);
  const kind = status.data?.linear.kind;
  const offerKey = kind === "not_configured" || kind === "connect_failed";
  return (
    <StepFrame
      nav={nav}
      title={t("onboarding.title.linear")}
      description={t("onboarding.description.linear")}
      forward={{ label: t("onboarding.next"), disabled: kind !== "connected" }}
    >
      <LinearStatus manage labels={{ login: t("onboarding.authorize"), connecting: t("onboarding.checking") }} />
      {offerKey && (
        <Collapsible open={keyOpen} onOpenChange={setKeyOpen}>
          <CollapsibleTrigger asChild>
            <Button variant="ghost" size="sm" disabled={waiting} className="group/key -ml-2">
              {t("onboarding.use_api_key")}
              <ChevronDown aria-hidden className="transition-transform group-data-[state=open]/key:rotate-180" />
            </Button>
          </CollapsibleTrigger>
          <CollapsibleContent className="mt-3">
            <ApiKeyForm />
          </CollapsibleContent>
        </Collapsible>
      )}
    </StepFrame>
  );
}
