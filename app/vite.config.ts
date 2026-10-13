/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

// Port 1420 is what tauri.conf.json's devUrl points at; Tauri needs it fixed.
export default defineConfig(({ mode }) => ({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": path.resolve(import.meta.dirname, "src") } },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
    // Vite refuses to load a file outside this folder. The timeline tests read the executor streams
    // that core tests itself against (crates/core/tests/fixtures), so a test run may read the
    // repository; the dev server keeps Vite's default and serves only this folder.
    ...(mode === "test" ? { fs: { allow: [path.resolve(import.meta.dirname, "..")] } } : {}),
  },
  build: { target: "es2022", sourcemap: false },
  test: {
    // Global test/expect/vi, which also turns on Testing Library's cleanup after each test.
    globals: true,
    environment: "jsdom",
    setupFiles: ["./src/test-setup.ts"],
    // Only the component and unit tests: the Playwright specs under `tests/` (`*.spec.ts`) are not Vitest's.
    include: ["src/**/*.test.{ts,tsx}"],
    restoreMocks: true,
    // Vitest turns every CSS file into an empty string; styles.test.ts reads the design tokens as text.
    css: { include: [/styles\.css\?raw/] },
  },
}));
