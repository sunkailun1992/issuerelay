import { CircleCheck, KeyRound, LoaderCircle } from "lucide-react";
import { type FormEvent, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Field, Hint } from "@/components/FormField";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useLinearApiKey, useLinearLoginWaiting, useStatus } from "@/hooks/queries";

/**
 * Connecting to Linear with a personal API key instead of the browser login (design 7), for the
 * settings and the first-run guide. The key is typed into a password field and goes to the
 * engine once, with the blanks of a paste taken off; it is shown nowhere (the engine keeps it in
 * the system keychain) and the field is cleared as soon as the engine has answered, whether it
 * took the key or not; the action's own record of what it was called with goes with it. A refusal is told by the action's own failure message: a headline and
 * the engine's words.
 *
 * core does one change of the Linear login at a time, so while a browser login waits the key
 * waits too, and the form says why; using it then would only queue behind the login for up to ten
 * minutes.
 */
export function ApiKeyForm() {
  const { t } = useTranslation();
  const id = useId();
  const [key, setKey] = useState("");
  const waiting = useLinearLoginWaiting();
  const status = useStatus();
  const use = useLinearApiKey();
  const trimmed = key.trim();
  const canUse = trimmed !== "" && !waiting && !use.isPending;
  // What the connection is now, not what this visit did: it is true after a restart too.
  const usingKey = status.data?.linear.kind === "connected" && status.data.linear.method === "api_key";

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!canUse) return;
    use.mutate(trimmed, {
      onSettled: () => {
        // The key is gone from the field and from the action's record.
        setKey("");
        use.reset();
      },
    });
  };

  return (
    <form onSubmit={submit} className="flex flex-col gap-1.5">
      <Field label={t("settings.api_key")} htmlFor={id}>
        <div className="flex gap-2">
          <Input
            id={id}
            type="password"
            value={key}
            onChange={(event) => setKey(event.target.value)}
            autoComplete="off"
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            aria-describedby={`${id}-hint`}
            className="min-w-0 font-mono"
          />
          <Button type="submit" variant="outline" disabled={!canUse}>
            {use.isPending ? <LoaderCircle aria-hidden className="animate-spin" /> : <KeyRound aria-hidden />}
            {t("settings.use_key")}
          </Button>
        </div>
        <Hint id={`${id}-hint`}>{t("settings.api_key_hint")}</Hint>
      </Field>
      {waiting && <p className="text-xs text-muted-foreground">{t("settings.api_key_after_login")}</p>}
      {usingKey && (
        <p role="status" className="flex items-center gap-1.5 text-sm">
          <CircleCheck aria-hidden className="size-4 text-state-done" />
          {t("settings.api_key_used")}
        </p>
      )}
    </form>
  );
}
