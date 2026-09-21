import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = (globalThis as { process?: { env?: Record<string, string | undefined> } }).process?.env?.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    host: host ?? "127.0.0.1",
    port: 1420,
    strictPort: true,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
  },
});
