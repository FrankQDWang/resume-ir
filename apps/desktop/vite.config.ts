import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { defineConfig } from "vite"

// @ts-expect-error The Node-only Vite plugin is contract-tested as native ESM.
import { pdfJsAssets } from "./scripts/pdfjs-assets.mjs"

export default defineConfig({
  plugins: [react(), tailwindcss(), pdfJsAssets()],
  clearScreen: false,
  server: { host: "127.0.0.1", port: 1420, strictPort: true },
  envPrefix: ["VITE_", "TAURI_"],
  build: { target: "safari13", sourcemap: false },
})
