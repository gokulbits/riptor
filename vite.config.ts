import { relative, sep } from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;
const root = import.meta.dirname;

// Only the repo's own .claude: a "**/.claude/**" glob matches every file inside a worktree.
const inRootClaude = (path: string) => {
  const rel = relative(root, path);
  return rel === ".claude" || rel.startsWith(".claude" + sep);
};

// https://v2.tauri.app/start/frontend/vite/
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**", inRootClaude, "**/vendor/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    minify: !process.env.TAURI_ENV_DEBUG,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
  test: {
    environment: "node",
    // Matched against paths relative to the root, so this is safe in a worktree.
    exclude: ["**/node_modules/**", "**/dist/**", "**/.claude/**", "**/vendor/**"],
  },
});
