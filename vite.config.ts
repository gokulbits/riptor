import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;

// https://v2.tauri.app/start/frontend/vite/
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    // .claude/worktrees holds other sessions' checkouts; vendor/ has unrelated web assets.
    watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**", "**/.claude/**", "**/vendor/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    // Vite 8 (Rolldown) minifies with Oxc by default; just turn it off for debug builds.
    minify: !process.env.TAURI_ENV_DEBUG,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
  test: {
    environment: "node",
    // Don't pick up tests from other sessions' worktrees or vendored code.
    exclude: ["**/node_modules/**", "**/dist/**", "**/.claude/**", "**/vendor/**"],
  },
});
