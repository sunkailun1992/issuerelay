import { RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Field, Hint } from "@/components/FormField";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { useModelCatalog } from "@/hooks/queries";
import { catalogModels } from "@/lib/model-catalog";
import { isValidModelName } from "@/lib/model-name";
import type { ExecutorKind } from "@/lib/types";

type CatalogQuery = Pick<ReturnType<typeof useModelCatalog>, "data" | "isFetching" | "isError" | "refetch">;

/** Enter a model ID directly or fill the same field from the executor's model catalog. */
export function ModelCatalogField({
  id, label, executor, value, onChange, hint, catalog,
}: {
  id: string;
  label: string;
  executor: ExecutorKind;
  value: string;
  onChange: (value: string) => void;
  hint: string;
  catalog: CatalogQuery;
}) {
  const { t } = useTranslation();
  const model = value.trim();
  const invalid = model !== "" && !isValidModelName(model);
  const choices = catalogModels(catalog.data, executor, invalid ? undefined : model);
  const recommended = choices.filter((choice) => choice.recommended);
  const others = choices.filter((choice) => !choice.recommended);
  const hintId = `${id}-hint`;
  const refresh = t("model_catalog.refresh", { label });
  const status = catalog.isFetching
    ? t("model_catalog.loading")
    : catalog.isError
      ? t("model_catalog.failed")
      : catalog.data?.source === "builtin" && executor === "codex"
        ? t("model_catalog.fallback")
        : null;

  return (
    <Field label={label} htmlFor={id}>
      <Input
        id={id}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        aria-invalid={invalid}
        aria-describedby={hintId}
        autoComplete="off"
        autoCapitalize="off"
        spellCheck={false}
        className="font-mono"
      />
      <Hint id={hintId} tone={invalid ? "error" : "muted"}>{invalid ? t("bindings.model_invalid") : hint}</Hint>
      <div className="flex min-w-0 items-center gap-2">
        <Select value={invalid ? "" : model} onValueChange={onChange} disabled={choices.length === 0}>
          <SelectTrigger aria-label={t("model_catalog.choices", { label })} size="sm" className="min-w-0 flex-1">
            <SelectValue placeholder={t("model_catalog.choose")} />
          </SelectTrigger>
          <SelectContent position="popper" align="start">
            {recommended.length > 0 && (
              <SelectGroup>
                <SelectLabel>{t("model_catalog.recommended")}</SelectLabel>
                {recommended.map((choice) => <SelectItem key={choice.id} value={choice.id}>{choice.label}</SelectItem>)}
              </SelectGroup>
            )}
            {others.map((choice) => <SelectItem key={choice.id} value={choice.id}>{choice.label}</SelectItem>)}
          </SelectContent>
        </Select>
        {executor === "codex" && (
          <Button
            type="button"
            variant="outline"
            size="icon-sm"
            aria-label={refresh}
            title={refresh}
            disabled={catalog.isFetching}
            onClick={() => void catalog.refetch()}
          >
            <RefreshCw aria-hidden="true" />
          </Button>
        )}
      </div>
      {status !== null && <p role="status" className="text-xs text-muted-foreground">{status}</p>}
    </Field>
  );
}
