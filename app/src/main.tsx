import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'
import { initI18n, resolveLanguage } from './i18n'

// Temporary: there is no backend yet, so the browser's language stands in for the system language
// that Rust reports (`AppInfo.system_locale`); Task 5 switches to that value.
await initI18n(resolveLanguage('system', navigator.language))

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
