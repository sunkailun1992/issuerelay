import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import type { ReactElement } from "react";
import { MemoryRouter } from "react-router";
import { Toaster } from "@/components/ui/sonner";
import { initI18nOnce, type Lang } from "@/i18n";
import type { Backend } from "@/lib/backend";
import { BackendProvider } from "@/lib/backend-context";
import { createMockBackend } from "@/lib/mock";

interface RenderOptions {
  /** The backend the component talks to; the demo mock without delay by default. */
  backend?: Backend;
  /** The address the router starts at. */
  route?: string;
  lang?: Lang;
}

export async function renderWithProviders(
  ui: ReactElement,
  { backend = createMockBackend("demo", { latencyMs: 0 }), route = "/", lang = "en" }: RenderOptions = {},
) {
  // i18next was started by the test setup (with the missing-key guard); this only switches
  // the language.
  await initI18nOnce(lang);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const result = render(
    <BackendProvider backend={backend}>
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={[route]}>{ui}</MemoryRouter>
        <Toaster />
      </QueryClientProvider>
    </BackendProvider>,
  );
  return { ...result, backend, client };
}
