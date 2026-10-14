import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";
import App from "./App";
import { initI18n, resolveLanguage } from "./i18n";
import { loadBackend } from "./lib/api";

const backend = await loadBackend();
// The interface starts in the system's language as Rust reports it (the tray and the
// notifications use the same value). The app info does not wait for the host, so this is quick;
// without it the panel still opens, in English, and shows what it cannot read as errors.
const info = await backend.appInfo().catch((error: unknown) => {
  console.error("could not read the app info", error);
  return null;
});
await initI18n(resolveLanguage("system", info?.system_locale ?? undefined));

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App backend={backend} />
  </StrictMode>,
);
